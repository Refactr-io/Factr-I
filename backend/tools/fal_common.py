"""Shared FAL.ai SDK plumbing: the lazy ``fal_client`` import.

Stateful pieces (cache globals, ``_submit_fal_request``) intentionally stay on
:mod:`tools.image_generation_tool`: it is the patch target for the test suites and for
``plugins/image_gen/fal/``'s ``_it`` indirection.
"""

from __future__ import annotations

from typing import Any


def import_fal_client() -> Any:
    """Import ``fal_client`` (via ``lazy_deps`` when available); raises ImportError if unavailable.

    Callers cache the result on their own module global so tests can monkeypatch it.
    """
    try:
        from tools.lazy_deps import ensure as _lazy_ensure
        _lazy_ensure("image.fal", prompt=False)
    except ImportError:
        pass
    except Exception as exc:  # noqa: BLE001 — lazy_deps surfaces install hints
        raise ImportError(str(exc))
    import fal_client  # type: ignore  # noqa: WPS433 — intentionally lazy
    return fal_client
