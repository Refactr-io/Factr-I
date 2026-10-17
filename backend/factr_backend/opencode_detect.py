"""Detect an installed OpenCode and adopt its Zen / Go keys on an explicit user action.

Detection only stats paths. OpenCode's own ``auth.json`` is opened solely by :func:`adopt`, which
the user triggers (one click), and the key values go straight into the app's secret store: they are
never logged, returned or cached anywhere else.

The keyless ``/models`` listing is public, but inference is not (the free tier answers
``FreeTierError`` outside OpenCode), so a detected install still needs a key to chat.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import threading
import time
from pathlib import Path
from typing import Any

ZEN_MODELS_URL = "https://opencode.ai/zen/v1/models"
_CACHE_TTL_S = 6 * 3600
# OpenCode auth.json provider id -> (Factr provider slug, env var)
_AUTH_IDS = {"opencode": ("opencode-zen", "OPENCODE_ZEN_API_KEY"), "opencode-go": ("opencode-go", "OPENCODE_GO_API_KEY")}
_refreshing = threading.Event()


def _home() -> Path:
    return Path(os.path.expanduser("~"))


def _data_dir() -> Path:
    return Path(os.environ.get("XDG_DATA_HOME") or _home() / ".local" / "share") / "opencode"


def _config_dir() -> Path:
    return Path(os.environ.get("XDG_CONFIG_HOME") or _home() / ".config") / "opencode"


def _binary_candidates() -> list[Path]:
    home = _home()
    names = ("opencode.exe", "opencode.cmd") if sys.platform == "win32" else ("opencode",)
    dirs = [home / ".opencode" / "bin", home / ".npm-global" / "bin", home / ".bun" / "bin",
            Path("/opt/homebrew/bin"), Path("/usr/local/bin"), Path("/home/linuxbrew/.linuxbrew/bin")]
    local = os.environ.get("LOCALAPPDATA")
    if local:
        dirs.append(Path(local) / "opencode")
    return [d / n for d in dirs for n in names]


def detect() -> dict[str, Any]:
    """``{installed, via, auth_present}``; no file content is read."""
    via = ""
    if os.environ.get("FACTR_OPENCODE_DETECT", "").lower() in ("0", "off", "false"):
        pass
    elif shutil.which("opencode"):
        via = "path"
    elif any(p.is_file() for p in _binary_candidates()):
        via = "binary"
    elif any(p.exists() for p in (Path("/Applications/OpenCode.app"), _home() / "Applications" / "OpenCode.app")):
        via = "app"
    elif _config_dir().is_dir() or _data_dir().is_dir():
        via = "config"
    return {"installed": bool(via), "via": via, "auth_present": (_data_dir() / "auth.json").is_file()}


def parse_models(payload: Any) -> list[str]:
    """Ids from an OpenAI-style ``{"data": [{"id": ...}]}`` body; tolerant of junk, order kept, deduped."""
    rows = payload.get("data") if isinstance(payload, dict) else None
    out: list[str] = []
    for row in rows if isinstance(rows, list) else []:
        mid = str(row.get("id", "")).strip() if isinstance(row, dict) else ""
        if mid and mid not in out:
            out.append(mid)
    try:
        from factr_backend.models import _OPENCODE_FREE_EXCLUDED_MODELS as dead
    except Exception:
        dead = frozenset()
    return [m for m in out if m.lower() not in dead]


def is_free(model_id: str) -> bool:
    return model_id.endswith("-free") or model_id == "big-pickle"


def _cache_path() -> Path:
    from factr_constants import get_factr_home
    return get_factr_home() / "cache" / "opencode-zen-public-models.json"


def _fetch_public() -> list[str]:
    import urllib.request
    req = urllib.request.Request(ZEN_MODELS_URL, headers={"Accept": "application/json", "User-Agent": "Factr-I"})
    with urllib.request.urlopen(req, timeout=6) as resp:
        return parse_models(json.loads(resp.read().decode("utf-8")))


def _refresh_cache() -> None:
    try:
        ids = _fetch_public()
        if ids:
            path = _cache_path()
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps({"at": time.time(), "ids": ids}))
    except Exception:
        pass
    finally:
        _refreshing.clear()


def public_models() -> list[str]:
    """Cached keyless Zen listing. Never blocks: a stale or missing cache warms in a background thread."""
    ids: list[str] = []
    fresh = False
    try:
        data = json.loads(_cache_path().read_text())
        ids = [str(m) for m in data.get("ids", [])]
        fresh = time.time() - float(data.get("at", 0)) < _CACHE_TTL_S
    except Exception:
        pass
    if not fresh and not _refreshing.is_set():
        _refreshing.set()
        threading.Thread(target=_refresh_cache, daemon=True, name="opencode-models").start()
    return ids


def _has_key(env_var: str) -> bool:
    try:
        from agent.credential_pool import get_env_prefer_dotenv
        return bool(get_env_prefer_dotenv(env_var) or os.environ.get(env_var))
    except Exception:
        return bool(os.environ.get(env_var))


def status() -> dict[str, Any]:
    """What the UI needs: install state, whether Zen/Go are already connected, public model counts."""
    info = detect()
    ids = public_models() if info["installed"] else []
    return {**info, "zen_connected": _has_key("OPENCODE_ZEN_API_KEY"), "go_connected": _has_key("OPENCODE_GO_API_KEY"),
            "models": ids, "free_models": [m for m in ids if is_free(m)]}


def adopt() -> dict[str, Any]:
    """Copy the user's OpenCode keys into the app's secret store. Returns slugs only, never values."""
    path = _data_dir() / "auth.json"
    try:
        store = json.loads(path.read_text())
    except FileNotFoundError:
        return {"adopted": [], "reason": "not_signed_in"}
    except Exception:
        return {"adopted": [], "reason": "unreadable"}
    from factr_backend.credential_lifecycle import save_provider_env_credential
    adopted: list[str] = []
    for auth_id, (slug, env_var) in _AUTH_IDS.items():
        entry = store.get(auth_id) if isinstance(store, dict) else None
        key = str(entry.get("key", "")).strip() if isinstance(entry, dict) and entry.get("type") == "api" else ""
        if not key:
            continue
        save_provider_env_credential(env_var, key)
        os.environ[env_var] = key
        adopted.append(slug)
    return {"adopted": adopted, "reason": "" if adopted else "no_key"}
