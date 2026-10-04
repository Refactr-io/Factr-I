"""Serve-start setup bootstrap: inventory which provider carries inference.

Every Factr process runs this once at boot (``factr serve`` on a daemon thread beside the other
background boots; the CLI first-run guard synchronously). It inventories credentials, resolves which
provider carries inference, records the answer in process memory, and tells every connected client
with one ``setup.ready`` event. ``setup.status`` reads the record. No network, no account creation.

The record keeps its ``has_identity`` / ``error`` / ``failure`` fields for wire compatibility;
they are always false / empty.
"""

from __future__ import annotations

import logging
import threading
import time
from dataclasses import asdict, dataclass, field, replace
from typing import Any, Dict, Optional

logger = logging.getLogger("factr_backend.auth")

# The desktop's first ``setup.status`` waits this long for the record before falling back to a live
# probe; the inventory is local and normally finishes well inside it.
SETUP_READY_WAIT_SECONDS = 8.0
SETUP_READY_EVENT = "setup.ready"


@dataclass(frozen=True)
class SetupRecord:
    """What the bootstrap found. One shape for every reader; no version field (renderer and backend
    ship together)."""

    provider_configured: bool      # some provider can carry inference
    inference_provider: str        # ``resolve_provider("auto")``'s answer, "" when nothing resolves
    has_identity: bool             # always False (kept for wire compatibility)
    other_providers: bool          # the inventory found something usable
    error: str = ""                # always "" (kept for wire compatibility)
    # Always ``{}``; kept so every status RPC can spread it as is.
    failure: Dict[str, Any] = field(default_factory=dict)
    finished_at: float = field(default_factory=time.time)

    def as_payload(self) -> Dict[str, Any]:
        # The broadcast carries the failure block flat, the same shape ``setup.status`` spreads,
        # so a client keys on ``error_code`` identically whichever surface it read.
        payload = asdict(self)
        payload.update(payload.pop("failure"))
        return payload

    def failure_fields(self) -> Dict[str, Any]:
        return dict(self.failure)


_lock = threading.Lock()
_record: Optional[SetupRecord] = None
_done = threading.Event()
_started = False
# ``(mtime_ns, size)`` of the files the inventory reads, taken by the inventory that built the
# current record; ``reconcile_record`` re-inventories only when they moved.
_inventory_stamp: Optional[tuple] = None
_INVENTORY_FILES = ("config.yaml", ".env", "auth.json")


def current_record() -> Optional[SetupRecord]:
    """The record, or None until the first bootstrap finishes."""
    return _record


def wait_for_record(timeout: float = SETUP_READY_WAIT_SECONDS) -> Optional[SetupRecord]:
    """Block up to ``timeout`` seconds for a bootstrap that is IN FLIGHT, then return whatever it
    produced, reconciled with any provider configured since (:func:`reconcile_record`). Returns
    None at once when no bootstrap ever started in this process (a bare ``tui_gateway`` under
    test, an old serve without the boot hook): the caller falls back to its live probe instead of
    paying the wait for nothing."""
    if not _started:
        return None
    _done.wait(timeout)
    return reconcile_record()


def reconcile_record() -> Optional[SetupRecord]:
    """Let a provider configured AFTER boot count: a record that says ``provider_configured:
    false`` is re-inventoried once ``config.yaml`` / ``.env`` / ``auth.json`` moved since the
    inventory that built it, and replaced (+ ``setup.ready``) when something now carries
    inference. A record that already says ``True`` is never re-probed, so the answer
    only moves false -> true here. Every write path that assigns the main model (the Models page,
    a picker key save) calls this for the immediate broadcast; ``setup.status`` calls it for
    writes this process never saw (``factr setup`` / ``factr model`` from a shell, a hand edit).
    The record is the LAUNCH profile's: a call scoped to another profile's home (a dashboard
    write with ``?profile=B``) leaves it alone, or B's providers would open the launch gate."""
    global _record
    record = _record
    if record is None or record.provider_configured:
        return record
    from factr_constants import get_process_factr_home, factr_home_key
    if factr_home_key() != factr_home_key(get_process_factr_home()) or _inventory_stamp == _config_stamp():
        return record
    if not _inventory_other_providers():
        return _record
    refreshed = replace(record, provider_configured=True, other_providers=True,
                        inference_provider=_resolve_inference(), finished_at=time.time())
    with _lock:
        if _record is not record:  # another inventory replaced it meanwhile; it is newer
            return _record
        _record = refreshed
    _broadcast(refreshed)
    return refreshed


def reset_for_tests() -> None:
    global _record, _started, _inventory_stamp
    with _lock:
        _record = None
        _started = False
        _inventory_stamp = None
        _done.clear()


def _config_stamp() -> tuple:
    from factr_backend.config import get_factr_home
    home = get_factr_home()
    stamp = []
    for name in _INVENTORY_FILES:
        try:
            st = (home / name).stat()
            stamp.append((st.st_mtime_ns, st.st_size))
        except OSError:
            stamp.append(None)
    return tuple(stamp)


def _inventory_other_providers() -> bool:
    """Is anything usable configured? Asks the resolver ladder itself (the thing that picks the
    provider for a turn): an explicit key, a config pin, a sign-in or a host credential answers;
    nothing else falls through to ``no_provider_configured``. Not ``_has_any_provider_configured``:
    that first-run guard also counts host credentials (gh auth, Claude Code).

    Stamps the config files BEFORE reading them, so a write that lands during the inventory is
    seen by the next :func:`reconcile_record`.
    """
    global _inventory_stamp
    from factr_backend.auth import resolve_provider
    _inventory_stamp = _config_stamp()
    try:
        return bool(resolve_provider("auto"))
    except Exception as exc:
        logger.debug("setup bootstrap: nothing carries inference (%s)", exc)
        return False


def _resolve_inference() -> str:
    from factr_backend.auth import resolve_provider
    try:
        return str(resolve_provider("auto") or "")
    except Exception:
        return ""


def _build_record(*, other: bool) -> SetupRecord:
    """One inventory pass into a record."""
    return SetupRecord(
        provider_configured=other,
        inference_provider=_resolve_inference(),
        has_identity=False,
        other_providers=other,
    )


def run_setup_inventory(*, announce: bool = True) -> SetupRecord:
    """Inventory -> resolve inference -> record -> broadcast.

    Runs every boot. Idempotent per process: a second call returns the existing record. Never
    raises. ``announce=False`` skips the
    ``setup.ready`` event: the plain CLI has no client to tell and its stdout is the user's terminal.
    """
    global _record, _started
    with _lock:
        if _record is not None:
            return _record
        if _started:
            _done.wait(SETUP_READY_WAIT_SECONDS)
            if _record is not None:
                return _record
        _started = True

    record = _build_record(other=_inventory_other_providers())
    with _lock:
        _record = record
        _done.set()
    if announce:
        _broadcast(record)
    return record


def _broadcast(record: SetupRecord) -> None:
    try:
        from tui_gateway.server import _broadcast_global_event
        _broadcast_global_event(SETUP_READY_EVENT, record.as_payload())
    except Exception as exc:  # no serve process (plain CLI): nobody to tell
        logger.debug("setup.ready not broadcast: %s", exc)


def start_background_inventory() -> threading.Thread:
    """``factr serve`` entry: run on a daemon thread so a slow inventory never delays the socket."""
    thread = threading.Thread(target=run_setup_inventory, daemon=True, name="setup-bootstrap")
    thread.start()
    return thread
