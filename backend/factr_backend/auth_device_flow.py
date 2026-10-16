"""Shared device-code / loopback-PKCE / browser / TLS helpers for interactive OAuth logins.

Split out of ``factr_backend/auth.py`` and re-exported there; origin helpers are imported lazily
inside each function so ``factr_backend.auth.<name>`` patches still intercept (and no import cycle).
"""

from __future__ import annotations

import base64
import hashlib
import logging
import os
import ssl
import sys
import threading
import time
import webbrowser
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Any, Callable, Dict, FrozenSet, Optional
from urllib.parse import parse_qs, urlparse
from factr_backend.auth_constants import (
    AuthError, OAUTH_OVER_SSH_DOCS_URL, httpx)
from utils import is_truthy_value

# Log-record parity with the origin module (caplog tests pin "factr_backend.auth").
logger = logging.getLogger("factr_backend.auth")

# Console/text-mode browsers that ``webbrowser`` will launch INSIDE the terminal, hijacking the
# user's TTY with an unusable text browser. When the resolved browser is one of these we refuse
# to auto-open and fall back to the print-the-URL path, same as a remote session.
_CONSOLE_BROWSER_NAMES: FrozenSet[str] = frozenset({
    "w3m", "lynx", "links", "links2", "elinks", "www-browser",
    "browsh",  # TUI browser — still hijacks the terminal
})

# Browser-only remote IDEs / cloud shells (they don't set SSH_CLIENT / SSH_TTY). Keep this list
# narrow — well-known env vars set by the host platform — so a local shell never trips it.
_REMOTE_IDE_ENV_VARS = (
    "CLOUD_SHELL",  # GCP Cloud Shell
    "CODESPACES", "CODESPACE_NAME",  # GitHub Codespaces
    "GITPOD_WORKSPACE_ID",  # Gitpod
    "REPL_ID",  # Replit
    "STACKBLITZ",  # StackBlitz
)


def _is_remote_session() -> bool:
    """Detect environments where loopback OAuth can't reach the local browser.

    Historically only SSH was checked, but #26923 surfaced that **browser-only remote consoles** (GCP Cloud
    Shell, GitHub Codespaces, AWS EC2 Instance Connect, Gitpod, Replit, etc.) hit the exact same problem —
    the user has a browser on their laptop but the loopback listener is bound on the remote VM that the
    laptop's browser can't reach. These environments typically don't set ``SSH_CLIENT`` / ``SSH_TTY``, so
    the SSH-only check left them with no guidance and no fallback.
    """
    return bool(
        os.getenv("SSH_CLIENT") or os.getenv("SSH_TTY")
        or any(os.getenv(var) for var in _REMOTE_IDE_ENV_VARS))


def _names_console_browser(value: str) -> bool:
    token = value.strip().split()[0] if value.strip() else ""
    return os.path.basename(token).lower() in _CONSOLE_BROWSER_NAMES


def _can_open_graphical_browser() -> bool:
    """Return True only when a *graphical* browser is likely to open.

    On a headless Linux box ``webbrowser.open()`` often resolves to a text-mode browser that takes
    over the terminal. Heuristics: a ``$BROWSER`` naming a console browser refuses; on Linux a
    display server (``$DISPLAY`` / ``$WAYLAND_DISPLAY``) is required unless ``$BROWSER`` is set
    (a console one already returned False, so a set ``$BROWSER`` here is graphical).
    """
    browser_env = os.environ.get("BROWSER", "")
    if browser_env and _names_console_browser(browser_env):
        return False
    if sys.platform.startswith("linux"):
        has_display = bool(os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"))
        if not has_display and not browser_env:
            return False
    try:
        controller = webbrowser.get()
    except Exception:
        return False  # No browser resolvable at all → definitely don't auto-open.
    candidate = getattr(controller, "name", "") or getattr(controller, "basename", "") or ""
    return not (candidate and _names_console_browser(candidate))


def _ssh_user_at_host() -> str:
    """Best-effort 'user@hostname' for the SSH tunnel hint; placeholders keep it valid syntax."""
    try:
        import socket as _socket
        hostname = _socket.gethostname() or "<this-host>"
    except OSError:
        hostname = "<this-host>"
    user = os.getenv("USER") or os.getenv("LOGNAME") or "<user>"
    return f"{user}@{hostname}"


def _pkce_code_verifier(length: int = 64) -> str:
    return base64.urlsafe_b64encode(os.urandom(length)).decode("ascii").rstrip("=")[:128]


def _pkce_code_challenge(code_verifier: str) -> str:
    digest = hashlib.sha256(code_verifier.encode("utf-8")).digest()
    return base64.urlsafe_b64encode(digest).decode("ascii").rstrip("=")


# The Factr-I helmet mascot, drawn bare (no tile, outline only) in Rose Alloy on the Charcoal Surface page; same art as the app. The tab
# icon is the tiled app icon at 32px, inlined so the page makes no external request.
_HELMET_ICON_SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="137 101 741 820" width="741" height="820">'
    '<path fill="#FFFDFC" fill-rule="evenodd" d="M580.8 101.0c-3.4 0.1 -8.5 0.3 -11.3 0.5c-2.8 0.2 -6.6 0.5 -8.5 0.6c-1.8 0.1 -4.4 0.3 -5.7 0.5c-5.2 0.5 -11.7 1.2 -13.3 1.3c-0.9 0.0 -2.6 0.3 -3.7 0.5c-1.1 0.2 -3.4 0.5 -5.2 0.7c-5.9 0.7 -16.1 2.3 -21.7 3.5c-3.0 0.6 -6.4 1.2 -7.6 1.5c-1.1 0.2 -3.8 0.7 -5.8 1.2c-2.0 0.4 -5.2 1.2 -7.1 1.6c-7.6 1.7 -11.1 2.5 -15.5 3.7c-2.5 0.7 -5.8 1.5 -7.2 1.9c-10.5 2.7 -30.9 9.5 -41.2 13.6c-1.8 0.7 -4.1 1.6 -5 1.9c-1.8 0.7 -5.2 2.1 -8.5 3.5c-1.1 0.5 -3.8 1.6 -6.0 2.5c-3.3 1.3 -8.1 3.4 -11.6 5.1c-0.5 0.2 -3.9 1.8 -7.5 3.4c-3.5 1.6 -7.2 3.3 -8.1 3.8c-14.3 7.4 -15.3 8.0 -22.7 12.3c-0.7 0.4 -2.5 1.4 -3.8 2.2c-2.6 1.5 -7.7 4.5 -12.2 7.3c-1.5 0.9 -5.0 3.0 -7.8 4.8c-6.0 3.6 -7.8 4.8 -15.2 10c-3.1 2.1 -8.2 5.7 -11.5 8c-9.8 6.8 -13.0 9.3 -25.5 19.4c-9.1 7.3 -22.2 19.1 -31.0 27.8c-7.1 7.0 -16.7 17.0 -20.4 21.3c-1.4 1.6 -3.4 4 -4.5 5.2c-4.7 5.5 -7 8.2 -10.9 13.3c-2.3 3 -4.8 6.4 -5.7 7.5c-0.8 1.1 -2.1 3.0 -3.0 4.2c-4.5 6.1 -15.1 22.2 -18.0 27.2c-1 1.7 -2.1 3.6 -2.5 4.2c-1.4 2.3 -7.1 13.0 -10 18.6c-2.1 4.1 -6.3 13.2 -7.5 16.1c-0.4 1.1 -1.6 3.8 -2.5 6.0c-3.7 8.8 -7.6 19.5 -9.1 25.2c-0.2 0.9 -0.8 2.8 -1.2 4.2c-0.7 2.4 -1.9 6.9 -3.3 12.2c-0.7 2.6 -1.3 5.3 -2.1 9.8c-2.3 12.2 -3.0 16.8 -3.8 26.3c-0.3 3.7 -0.7 8.8 -1.0 11.3c-0.7 8.1 -0.2 27.3 1 40.6c0.7 7.7 1.8 15.8 2.6 20.5c0.2 1.5 0.6 3.8 0.8 5c0.1 1.2 0.6 3.5 1 5.3c1.8 8.6 2.9 13.3 3.5 15.6c0.3 1.3 0.9 3.8 1.4 5.5c0.4 1.6 1.2 4.4 1.7 6.1c1.0 3.3 1.1 4.2 0.6 5.2c-0.1 0.3 -1.2 2.3 -2.3 4.3c-3.6 6.6 -5.5 10.9 -8 18.4c-2.5 7.5 -4.3 13.7 -4.7 16.5c-0.1 0.9 -0.5 2.5 -0.7 3.7c-0.2 1.1 -0.7 3.8 -1 6c-0.2 2.1 -0.7 5.0 -1 6.5c-0.2 1.4 -0.6 4.3 -0.7 6.3c-0.1 2.0 -0.4 5.8 -0.6 8.3c-0.4 5.3 -0.5 28.3 -0.1 30.2c0.1 0.6 0.3 2.4 0.5 4c0.6 6.7 2 15.3 3.4 21.2c1.7 7.3 4.6 16.7 6.6 21.0c0.4 0.9 0.7 1.6 0.7 1.8c0 0.2 3.6 7.5 4.6 9.4c5.4 9.8 8.4 14.5 14.0 21.5c5.0 6.2 20.0 21.3 26.7 26.8c3.7 3.1 16.0 12.8 19.9 15.8c10.0 7.7 25.8 18.6 33.7 23.3c1.8 1.1 5.4 3.4 8 5.1c5.3 3.5 7.3 4.7 14.3 8.6c2.6 1.4 6.4 3.5 8.4 4.6c5.6 3.1 14.7 7.7 21.6 10.9c10.0 4.7 13.7 6.0 20.4 7.4l4.3 0.9l8.9 0.0l8.9 0.0l0.9 0.6c0.5 0.3 2.5 1.7 4.4 3.0c4.6 3.2 8.5 5.7 12.7 8.2c20.1 11.9 24.6 14.5 29.8 17.4c3.0 1.6 7.6 4.2 10.5 6c0.6 0.3 2.7 1.5 4.7 2.6c2 1.0 3.8 2.0 4.0 2.1c3.5 1.9 24.8 12.5 26.3 13.1c1.1 0.4 3.2 1.3 4.7 1.9c4.9 2.1 11.0 3.8 15.7 4.5c1.4 0.2 3.6 0.5 5.0 0.8c1.3 0.2 4.0 0.6 5.8 0.8c1.8 0.2 6.3 0.7 10 1.1c3.6 0.4 7.7 0.9 9.1 1.1c1.3 0.2 4.5 0.6 7.1 0.8c2.5 0.3 7.9 0.9 11.8 1.3c4 0.4 8.1 0.9 9.1 1c1.0 0.0 3.3 0.3 5.1 0.5c1.8 0.2 6.0 0.6 9.5 0.8c8.6 0.7 20.4 1.6 25.6 2.1c2.4 0.2 6.6 0.5 9.4 0.7c2.7 0.2 5.3 0.4 5.7 0.5c0.8 0.1 18.4 1.3 24.2 1.6c2.3 0.1 5.6 0.3 7.2 0.4c4.4 0.3 16.0 0.7 35.2 1.0c18.1 0.3 28.0 0.1 34.4-0.4c7.5-0.7 15.3-2.6 22.4-5.5c6.6-2.7 16.6-8.6 23.4-13.7c2.5-1.9 3-2.0 4.5-1.4c13.5 5.3 24 6.6 36.4 4.7c5.9-0.9 10.7-2.2 17.6-5.0c4.1-1.6 13.1-6.0 17.3-8.5c8.0-4.6 18.3-12.2 25.7-19.1c3.9-3.6 13.7-13.8 16.7-17.4c2.7-3.2 7.5-9.6 9.2-12.1c1-1.5 4.2-6.7 5.7-9.3c0.4-0.7 1.4-2.4 2.1-3.7c4-6.9 8.2-16.3 11.7-25.8c3.2-9.1 5.3-16.1 7.1-24.7c0.3-1.3 0.9-4.0 1.3-5.8c0.6-3.2 1.9-11.5 2.4-16.7c0.3-2.9 1.1-13.8 1.5-19.7c0.1-2.4 0.3-5.9 0.5-7.8c0.1-1.9 0.3-13.3 0.3-25.3c0.2-39.4 -0.4-48.3 -4.3-60.3c-1.2-3.7 -2.3-6.3 -4.2-10.2c-1.7-3.5 -1.9-4.4 -1.1-5.9c3.0-5.2 5.5-13.2 6.3-19.8c0.3-2.8 -0.0-17.2 -0.5-20.7c-0.1-1.0 -0.4-3.6 -0.7-5.8c-1-8.1 -4.4-23.1 -6.2-27.5c-0.5-1.2 -1.2-3.7 -3.1-10.7c-0.7-2.9 -1.7-6.3 -2.1-7.6c-0.4-1.3 -1.3-4.3 -2.0-6.6c-2.3-7.6 -2.6-8.7 -3.5-11.2c-0.5-1.3 -1.3-3.6 -1.9-5.1c-0.5-1.4 -1.3-3.5 -1.7-4.6c-1.4-3.8 -2.6-7.2 -3.4-9.1c-0.4-1.0 -1.5-3.6 -2.4-5.7c-0.8-2.1 -1.9-4.6 -2.3-5.6c-1.1-2.8 -7.6-15.7 -10.5-20.9c-2.3-4.2 -4.7-8.4 -8.2-14c-6.2-10.0 -15.9-23.4 -23.6-32.2c-1.6-1.9 -3.7-4.3 -4.6-5.3c-2.6-3.1 -13.2-13.7 -16.6-16.8c-8.7-7.8 -18.0-15.5 -23.2-19.2c-5.6-4.0 -16.1-11.1 -17.6-11.9c-0.3-0.1 -1.8-1.0 -3.3-1.9c-6.3-3.8 -10.5-6.2 -15.8-9.0c-1.0-0.5 -2.8-1.5 -4-2.2c-1.1-0.6 -3.7-1.9 -5.6-2.8c-6.6-3.0 -10.7-4.8 -12.6-5.6c-3.2-1.2 -3.4-1.5 -3.4-4.2c0-2.6 0.9-15.5 1.2-17.0c0.1-0.7 0.4-2.8 0.6-4.6c1.1-9.5 5.4-27.0 9.3-37.9c0.6-1.8 1.6-4.5 2.1-5.9c2.5-7.2 7.5-18.9 10.6-24.8c6.1-11.5 10.5-19 15.1-25.5c1.4-2.0 3.6-5.2 4.9-7c1.2-1.8 3.3-4.7 4.7-6.6c2.3-3.1 4.1-5.3 10.6-13.4c6-7.4 8.6-11.7 10.3-16.9c3.3-10.3 1.6-21.8 -4.3-29.8c-2.1-2.8 -6.0-6.7 -8.9-8.9c-11.1-8.1 -17.2-11.6 -30.9-17.5c-1.1-0.4 -2.6-1.1 -3.3-1.4c-3.9-1.7 -14.3-5.5 -19.2-6.9c-5.9-1.8 -9.5-2.7 -15.7-4.1c-1.7-0.3 -4.8-1.0 -7-1.5c-8.0-1.8 -16.2-3.3 -28.3-5.1c-16.3-2.4 -41.8-3.4 -65.7-2.7zm37.7 26.9c1.2 0.1 4.5 0.3 7.1 0.5c9.4 0.5 19.4 1.5 28.2 2.8c1.3 0.1 3.5 0.5 5 0.7c4.3 0.6 14.0 2.3 18.6 3.2c4.3 0.8 7.5 1.7 15.1 3.9c2.4 0.7 5.3 1.5 6.5 1.8c1.1 0.3 2.7 0.8 3.5 1.0c0.7 0.2 2.7 0.9 4.3 1.4c3.0 0.9 8.1 2.8 12.2 4.4c1.2 0.5 2.8 1.1 3.5 1.4c2.6 1.0 9.6 4.7 12.2 6.5c3.6 2.4 5.9 4.7 7.2 7.2c1.2 2.4 1.5 4 1.3 6.8c-0.1 1.9 -0.2 2.6 -1.1 4.7c-1.7 4.1 -1.8 4.2 -9.1 12.6c-2.0 2.3 -4.1 4.7 -4.6 5.3c-2.2 2.6 -14.4 19.1 -17.6 23.9c-9.3 13.9 -17.4 29.2 -22.8 42.6c-8.5 21.2 -13.9 42.5 -15.6 62.6c-0.0 0.9 -0.3 3.4 -0.5 5.6c-0.8 9.1 -0.0 27.9 1.5 37.2c0.3 1.9 0.7 4.5 0.8 5.8c0.1 1.2 0.4 3.2 0.7 4.3c0.6 2.5 1.5 7.8 1.5 8.7c0 1.9 -2.8 3.8 -6.5 4.4c-1.9 0.2 -6.9 0.1 -10-0.2c-0.9-0.1 -3.6-0.4 -6-0.6c-2.4-0.2 -5.9-0.6 -7.8-1c-1.9-0.3 -4.4-0.7 -5.5-0.8c-3.8-0.5 -16.1-3.8 -23.8-6.4c-11.1-3.6 -15.6-6.5 -23.4-14.7c-6.1-6.5 -11.1-11.5 -13.6-13.6c-15.5-13.4 -29.9-17.5 -62.1-17.5c-16.2 0 -28.1 0.8 -42.3 3.1c-1.6 0.2 -4.7 0.6 -6.7 0.9c-5.6 0.7 -10.6 1.6 -18.9 3.2c-2.9 0.6 -9.0 2.0 -12.2 2.8c-1.2 0.3 -2.7 0.7 -3.5 0.8c-3.4 0.7 -13.5 3.6 -21 6.0c-3.4 1.1 -7.1 2.2 -8.2 2.6c-3.3 1 -9.1 3.2 -19.2 7.3c-2.0 0.8 -4.6 1.8 -5.7 2.3c-3.1 1.2 -7.3 3.0 -17.1 7.5c-5.7 2.6 -6.6 2.4 -7.9-1.5c-1.1-3.4 -1.7-8.1 -1.7-12.8c0-4.4 0.2-6.3 1.1-9.8c0.3-1.3 1.1-4.8 1.8-7.6c2.1-9 5-16.4 10.2-27.1c3.2-6.6 7.7-14.7 10.3-18.8c8.2-13.0 15.1-22.5 23.8-32.6c4.1-4.7 17.8-19.7 18.6-20.2c0.2-0.1 3.1-2.7 6.5-5.8c3.4-3.1 7.4-6.6 8.9-7.9c2.5-2.0 4.2-3.3 11.2-8.7c1.1-0.8 3.3-2.5 5-3.8c2.5-1.9 8.2-5.8 13.7-9.4c11.0-7.1 11.3-7.3 11.3-8.2c0-1.1 -0.5-1.1 -7.7 0.0c-9.8 1.5 -18.4 3.3 -24 5.0c-1.1 0.3 -2.6 0.7 -3.3 0.9c-1.5 0.3 -11.8 3.3 -15.6 4.5c-5 1.5 -15.3 5.1 -18.7 6.5c-2.3 1 -8.5 3.4 -11 4.3c-1.9 0.7 -7 2.8 -14.6 6.1c-7.3 3.1 -21.3 9.6 -27.2 12.7c-11.2 5.7 -19.8 10.2 -22.8 11.8c-8.2 4.4 -16.1 7.7 -18.1 7.4c-0.6-0.0 -0.7-0.2 -0.7-1.0c-0.2-2.2 2.1-7.9 4.6-11.5c4.2-6 13.9-15.7 26.5-26.7c6.0-5.2 17.3-13.6 23.6-17.5c1.6-1.0 5.5-3.5 8.5-5.4c3.0-1.9 7.4-4.6 9.7-6.0c4-2.3 6.1-3.5 15.7-8.6c5.3-2.7 12.2-6.1 17-8.2c1.1-0.5 3.7-1.6 5.8-2.6c6.2-2.8 8.2-3.6 10.7-4.6c2.6-1.0 6.0-2.4 10.1-4.0c3.1-1.2 5.4-2.1 12.5-4.6c9.9-3.5 31.5-10.2 34.6-10.8c0.4-0.1 1.8-0.4 3-0.7c1.1-0.3 3.6-0.9 5.5-1.3c1.8-0.4 5.1-1.2 7.3-1.7c11.7-2.8 36.4-7.1 50.2-8.6c7.7-0.8 7.6-0.8 14.2-1.2c3.1-0.2 7.1-0.4 8.8-0.6c1.7-0.1 5.3-0.3 8-0.4c7.3-0.3 27.2-0.3 29.8-0.0zm-319 150.1c2.1 1.0 2.7 3.4 1.5 6.7c-1.7 4.8 -10.1 15.4 -24.1 30.3c-2.8 3.0 -5.9 6.4 -6.9 7.5c-0.9 1.1 -3.6 3.9 -5.8 6.3c-2.2 2.4 -4.9 5.3 -5.9 6.5c-1.0 1.1 -3.3 3.7 -5.0 5.6c-1.7 1.9 -6.2 7.1 -10 11.6c-9.6 11.4 -11.4 13.7 -20.5 25.1c-1.1 1.3 -3.0 3.7 -4.2 5.2c-2.9 3.5 -3.2 3.9 -7.5 10.5c-1.9 3.0 -3.9 5.7 -4.2 6.0c-0.4 0.4 -0.8 0.5 -1.4 0.5c-1.3-0.1 -1.5-1.0 -1.5-8.3c0-4.9 0.1-6.8 0.4-8.8c1.1-6.5 2.5-11.6 4.9-18.2c0.6-1.8 1.5-4.2 1.9-5.2c1.0-3.0 6.1-13.0 8.6-17.2c3.6-5.9 8.8-13.9 10.1-15.6c0.6-0.8 1.7-2.2 2.4-3.0c2.5-3.3 12.6-13.9 17.5-18.4c8.6-7.9 17.4-14.3 27.9-20.5c7.0-4.1 12.2-6.3 16.2-6.9c1.7-0.2 4.7-0.0 5.7 0.4zm238.6 72.7c5.5 0.2 13.6 1.6 18.1 3.0c7.5 2.4 12.6 6.5 27 22.0c6.3 6.8 8.9 8.7 15.2 11.4c3.6 1.5 9.9 3.6 14.6 4.8c15.0 3.7 30.4 6.7 41.7 8.1c0.7 0.0 2.3 0.3 3.5 0.5c3.2 0.6 14.6 0.6 18.1-0.0c8.3-1.5 13.3-5.5 15.2-12.0c0.4-1.6 0.4-5.4 -0.2-10.6c-0.5-4.2 -0.7-9.9 -0.3-10.4c0.4-0.6 1.6-0.4 3.6 0.6c1.0 0.5 2.2 1.0 2.5 1.1c1 0.3 8.4 4 12.5 6.2c11.2 6.0 28.8 17.8 36.5 24.3c1.2 1.0 3.2 2.7 4.4 3.7c4.4 3.6 8.6 7.5 15.1 13.7c6.5 6.3 11.0 11.2 18.7 20.5c2.1 2.5 4.6 5.5 5.6 6.7c1 1.2 2.4 3 3.1 4.0c1.6 2.3 8.2 12.2 9.6 14.4c3.6 5.9 4.7 7.7 6.5 10.8c5.4 9.4 12.9 24.0 15.7 30.8c0.3 0.7 1 2.3 1.5 3.5c0.7 1.8 1.7 4.2 4.1 10.3c2.9 7.5 5.4 14.6 6.1 17.5c0.2 0.8 0.9 3.3 1.6 5.6c1.4 4.8 2.8 9.4 3.9 13.5c0.4 1.5 1.2 4.5 1.7 6.5c1.6 5.7 3.5 14.2 4.1 18.1c0.2 1.3 0.5 2.9 0.6 3.3c0.1 0.5 0.5 2.8 0.8 5.1c0.3 2.3 0.8 5.0 1 6c0.7 3.7 0.9 6.2 0.8 11.3c-0.1 5.1 -0.1 5.1 -1 7.6c-0.9 2.5 -2.4 5.5 -3.9 7.7c-1.1 1.7 -4.8 5.4 -7.8 8.0c-1.3 1.1 -2.5 2.2 -2.6 2.4c-0.3 1.0 0.1 1.5 3.0 2.9c6.6 3.2 8.8 6.0 11.1 14.3c1.1 4.1 1.6 7.5 2 12.4c0.4 6.3 0.4 51.3 0.0 57.5c-0.1 2.4 -0.4 7.2 -0.5 10.6c-0.1 3.3 -0.3 7.1 -0.5 8.3c-0.1 1.2 -0.3 3.5 -0.5 5.2c-0.1 1.6 -0.3 4.0 -0.4 5.3c-0.1 1.2 -0.3 3.3 -0.5 4.5c-0.3 3.0 -1.9 12.8 -2.3 14.9c-0.2 0.9 -0.6 2.9 -0.8 4.3c-1.6 9.4 -6.1 23.2 -10.6 33c-4.2 9.1 -6.3 12.9 -11.5 20.8c-4.8 7.3 -6.7 9.7 -13.5 16.8c-8.3 8.8 -19.0 17.6 -27.0 22.3c-0.9 0.5 -2.3 1.4 -3.2 1.9c-0.9 0.5 -2.0 1.2 -2.5 1.5c-8.2 5.1 -17.5 9.0 -25.1 10.7c-1.7 0.4 -5.7 0.4 -8.2 0.1c-2.2-0.3 -7-1.9 -7.7-2.6c-1-0.9 -0.8-1.7 1.1-4.6c1.7-2.6 5.2-8 6.8-10.5c1.6-2.6 3.8-6.2 5.6-9.4c0.9-1.7 2.3-4.0 2.9-5.2c4.9-8.5 12.7-24.9 17.4-36.8c0.8-2.2 2.0-5.1 2.6-6.5c3.4-8.2 7.0-18.8 9.0-26.3c0.5-2 1.2-5.5 1.5-7.8l0.5-4.2l-0.5-1.6c-0.8-2.7 -2.4-6.0 -4.4-9.5c-2.7-4.4 -4.8-8.9 -4.8-10.1c0-1.4 0.9-2.2 4.7-4.1c5.1-2.5 14.8-8.6 19.9-12.4c6.8-5.1 11.3-10.1 14-15.4c2.0-4.1 5-16.3 7.4-31c0.3-2.0 0.8-5 1.1-6.5c0.2-1.5 0.6-4 0.8-5.5c0.6-4.8 1.5-11.9 1.7-13.5c0.0-0.8 0.3-2.8 0.6-4.5c0.7-4.4 0.7-15.0 0.1-17.1c-0.4-1.2 -0.4-1.6 -0.1-2.5c0.6-2.4 1.6-3.9 6.2-9.3c3.6-4.2 4.8-5.9 5.5-8.2c2.3-7.1 -3.0-16.8 -12.9-23c-2.6-1.6 -8.3-4.1 -14.6-6.4c-3.9-1.4 -14.6-4.9 -18.5-6.0c-2-0.5 -4.4-1.2 -5.3-1.5c-7.9-2.3 -18-4.8 -33.5-8.5c-8.2-1.9 -12.1-2.9 -14.2-3.5c-1.2-0.3 -2.9-0.7 -3.7-0.8c-0.8-0.1 -2.5-0.5 -3.8-0.9c-4.3-1.1 -10.6-2.5 -12.5-2.9c-1.0-0.2 -5.0-1 -8.8-1.7c-3.8-0.7 -7.7-1.5 -8.6-1.7c-2.9-0.7 -23.4-4.6 -38.6-7.4c-3.4-0.6 -8.5-1.5 -11.2-2.1c-4.5-0.9 -12.5-2.2 -25.5-4.2c-2.7-0.4 -6.9-1.1 -9.3-1.5c-2.4-0.4 -5.6-0.9 -7.2-1.1c-1.5-0.2 -4.6-0.6 -6.8-1c-3.4-0.5 -21.8-3.1 -25.1-3.5c-0.6-0.0 -4-0.5 -7.5-1.1c-15.0-2.3 -19.3-2.8 -39.3-4.8c-3.2-0.3 -6.7-0.7 -7.7-0.8c-0.9-0.1 -3.4-0.4 -5.5-0.6c-2.0-0.2 -5.9-0.5 -8.6-0.8c-8.7-0.9 -24.0-2.2 -28.6-2.5c-2.4-0.1 -5.5-0.3 -6.8-0.5c-1.3-0.1 -4.4-0.3 -6.8-0.5c-2.4-0.1 -5.4-0.3 -6.5-0.5c-1.1-0.1 -4.1-0.3 -6.7-0.3c-2.6-0.0 -6.6-0.2 -8.8-0.5c-2.2-0.2 -6.8-0.4 -10.0-0.6c-13.2-0.6 -25.6 1.4 -37.1 6c-5.8 2.3 -13.4 7.7 -17.4 12.5c-2.4 2.9 -5.2 7.2 -6.9 10.8c-1.9 4 -2.6 5.9 -3.9 10.6c-0.9 3.3 -1.0 4.0 -1.2 8.6c-0.3 5.7 -0.1 37.7 0.2 42.3c0.2 2.8 0.7 8.3 1.2 13.7c0.2 2.6 1.4 8.3 2.7 14c2.3 9.6 8.1 16.9 21.1 26.7c8.9 6.7 13.5 9.5 25.8 15.7c3.5 1.7 7.4 3.7 8.6 4.4c1.2 0.7 4.2 2.1 6.7 3.2c2.4 1.1 4.9 2.2 5.4 2.5c8.3 3.9 14.8 6.4 21.7 8.3c3.4 0.9 12.0 2.8 21.5 4.7c1.6 0.3 3.8 0.8 4.8 1c1.0 0.2 4.0 0.7 6.7 1.2c2.6 0.5 7.1 1.3 10 1.9c2.8 0.5 7.3 1.3 10 1.8c9.5 1.7 14.1 2.5 17.1 3.1c1.6 0.3 4.6 0.8 6.6 1.2c3.3 0.6 21.3 3.8 33 5.9c2.6 0.4 7.9 1.3 11.6 1.9c3.7 0.6 8.4 1.4 10.5 1.7c2.0 0.3 6.4 1.0 9.6 1.6c3.2 0.5 7.6 1.3 9.7 1.6c2.1 0.3 4.0 0.6 4.2 0.7c0.2 0.0 4.4 0.7 9.3 1.5c16.9 2.6 18.5 2.8 21.2 3.3c1.5 0.2 7.4 1.1 13.2 2c5.7 0.8 10.6 1.5 10.8 1.6c0.2 0.0 2.8 0.4 5.7 0.7c2.9 0.3 6.3 0.8 7.5 1c1.1 0.2 3.8 0.5 5.8 0.7c2.0 0.2 5.1 0.6 6.8 0.8c1.7 0.2 5.1 0.7 7.5 1.0c2.4 0.2 5.5 0.6 7 0.8c6.3 0.9 15.1 1.8 23.5 2.4c5.4 0.4 12.4 0.3 15.8-0.1c5.4-0.7 5.4-0.7 6.0-0.2c0.4 0.3 0.7 1 0.9 2.0c0.2 0.8 0.8 2.9 1.4 4.6c3.1 8.7 4.6 13.3 4.6 14.4c0 0.6 -0.2 1.6 -0.6 2.5c-0.3 0.7 -1.2 2.9 -1.8 4.7c-0.6 1.8 -1.9 5.1 -2.8 7.3c-0.8 2.2 -2.3 5.9 -3.2 8.2c-0.9 2.3 -2.0 4.9 -2.4 5.7c-0.4 0.8 -0.8 1.9 -1.0 2.3c-0.3 0.9 -1.6 3.9 -3.9 9.5c-2.2 5.3 -11.7 24 -16.1 31.6c-1.7 3.1 -5.7 9.7 -7.8 12.8c-1.0 1.5 -2.7 4.3 -3.8 6.1c-2.2 3.8 -6.4 9.8 -9.2 13.4c-5.3 6.9 -8.1 9.4 -16.3 14.7c-6.2 3.9 -10.9 6.1 -16.1 7.5c-2.7 0.7 -3.4 0.7 -8.2 0.8c-3.1 0.0 -8.7-0.0 -13.8-0.2c-4.7-0.2 -11.6-0.5 -15.3-0.6c-3.6-0.1 -8.7-0.3 -11.1-0.5c-2.4-0.1 -7.1-0.3 -10.6-0.5c-3.4-0.1 -7.8-0.3 -9.7-0.5c-3.3-0.2 -7.2-0.5 -21.3-1.3c-3.3-0.2 -6.6-0.4 -7.3-0.5c-0.6-0.0 -3.3-0.2 -5.9-0.3c-2.5-0.1 -6.6-0.4 -9-0.6c-2.3-0.2 -7.6-0.6 -11.6-0.8c-4.0-0.2 -9.6-0.7 -12.5-1c-2.8-0.2 -7.6-0.6 -10.7-0.8c-3.1-0.2 -7.0-0.5 -8.7-0.7c-1.7-0.2 -5.3-0.5 -8.1-0.7c-5.4-0.4 -12.0-1.0 -12.6-1.2c-0.2-0.0 -2.5-0.3 -5.2-0.5c-2.6-0.2 -7.3-0.6 -10.3-1c-3.0-0.3 -7.0-0.7 -9-0.8c-1.9-0.1 -4.1-0.3 -5-0.5c-0.8-0.1 -3.9-0.5 -6.8-0.8c-9.1-1.0 -13.9-2.3 -19.4-4.9c-8.2-4 -27.5-14.2 -30.8-16.3c-0.4-0.3 -1.8-1.0 -3-1.6c-1.1-0.6 -3.4-1.8 -5.1-2.7c-1.6-0.9 -4.5-2.5 -6.5-3.6c-3.5-1.9 -7.6-4.3 -11.8-6.8c-1.3-0.8 -3.7-2.2 -5.5-3.2c-6.4-3.8 -7.4-4.3 -8.2-5.1c-0.6-0.6 -0.8-0.9 -0.6-1.3c0.0-0.3 1.3-1.7 2.8-3.0c6.9-6.4 8.0-7.8 12.3-14.6c4.1-6.5 5.8-10.0 9.0-18.3c0.6-1.6 1.3-3.5 1.6-4.1c1.9-4.6 7-21.9 7.8-26.7c0.2-1.1 0.6-2.9 0.8-4c0.2-1.0 0.9-4.3 1.4-7.3c0.9-5.0 1.2-7.2 2.2-14.1c0.2-1.5 0.6-4.4 0.8-6.5c1.1-8.3 1.1-10.9 -0.1-16.1c-1.7-7.3 -4.6-13.1 -8.7-17.6c-2.1-2.2 -7.2-7.0 -11.6-10.8c-1.8-1.5 -4.5-3.9 -5.9-5.1c-1.4-1.2 -4.3-3.7 -6.3-5.4c-2.0-1.7 -4.6-3.8 -5.6-4.6c-1.0-0.8 -3.1-2.6 -4.7-3.8c-1.5-1.2 -4.7-4.1 -7.1-6.2c-2.3-2.1 -5.8-5.3 -7.7-7.0c-7.9-7.1 -11.1-11.2 -12.8-16.4c-0.6-1.8 -2.3-10.7 -3.3-17.3c-0.3-2.3 -0.8-4.9 -1-5.8c-0.3-1.6 -0.8-4.6 -2-12.1c-0.8-5.6 -2.5-15.2 -3.2-19.1c-0.3-1.7 -0.7-4.2 -1-5.6c-0.6-3.9 -2.6-12.2 -3.6-15.2c-1.7-5.1 -4.3-10.0 -7.5-14.6c-6.9-9.7 -15.6-17.4 -26.8-23.5c-5.9-3.2 -16.5-6.5 -24.6-7.6c-4.2-0.5 -5.7-0.6 -12-0.6c-5.2 0 -8.0 0.1 -10.5 0.4c-1.8 0.2 -4.6 0.5 -6.1 0.7c-1.5 0.1 -3.9 0.5 -5.3 0.7c-5.9 1.1 -5.7 1.1 -6.1 0.7c-0.6-0.5 -0.4-1.9 0.2-3c0.3-0.5 1.1-2.1 1.7-3.6c0.6-1.4 1.2-2.9 1.3-3.1c0.1-0.2 1.2-2.6 2.5-5.3c1.2-2.7 2.6-5.6 3.0-6.5c3.6-7.5 13.5-23.6 18.5-30.1c2.1-2.6 2.9-3.7 7.3-8.9c3.5-4.1 13.2-14.3 17.6-18.5c7.9-7.5 23.9-20.3 31.7-25.6c6.2-4.1 10.3-6.7 12.3-8c1.1-0.6 3.7-2.2 5.7-3.5c2-1.2 5.4-3.3 7.6-4.5c2.2-1.2 4.9-2.7 6-3.3c1.1-0.6 3.9-2.1 6.3-3.3c2.4-1.2 5.3-2.7 6.5-3.3c1.1-0.6 3.3-1.6 4.7-2.3c9.8-4.5 16.1-7.2 18.3-8.1c0.8-0.3 2.4-0.9 3.5-1.4c6.7-2.9 7.7-3.3 13-5.3c8.5-3.1 16.1-5.7 21.7-7.4c1.6-0.4 4.3-1.2 6-1.7c7.1-2.0 11.6-3.2 20.6-5.1c2-0.4 4.7-1.0 6-1.3c1.3-0.3 4.1-0.9 6.2-1.3c2.1-0.4 4.3-0.8 4.8-0.9c0.5-0.1 2.4-0.4 4.1-0.6c1.7-0.2 4.2-0.6 5.6-0.8c1.3-0.2 3.4-0.5 4.5-0.6c1.1-0.0 3.4-0.3 5.1-0.6c1.7-0.2 4.7-0.6 6.6-0.8c6.0-0.6 10.2-1.1 12-1.3c6.0-0.8 27.4-1.3 37.6-0.9zm-271.8 192.6c3.1 0.2 3.8 0.3 7.6 1.5c6.4 2.0 10.8 4.3 15.9 8.3c4.9 3.8 8.6 8.0 11.3 12.8c1.7 3.1 2.6 5.3 3.7 9.6c1.1 4.5 4.5 21.2 6.6 33.2c0.4 2.4 1.0 6.0 1.4 7.8c0.8 4.7 2.0 12.1 2.5 15c0.2 1.3 0.5 3.1 0.7 4c0.2 0.9 0.5 3.0 0.8 4.7c1.7 10.8 5.3 17.1 14.5 25.3c4.3 3.8 11.1 9.7 15.0 12.9c1.7 1.3 4.7 4 6.8 5.7c2.0 1.7 5.9 4.9 8.6 6.9c10.3 8.0 16.2 12.9 19.3 16c2.4 2.5 4.6 5.6 6.1 9.1c1.5 3.6 1.6 2.7 -2.1 28.5c-1.0 7 -2.7 14.7 -4.8 22.3c-0.2 0.8 -0.7 2.9 -1.2 4.6c-1.1 4.0 -3.0 9.9 -4.6 14.2c-5.1 14.0 -8.3 19.6 -13.9 24.2c-5.8 4.9 -10.9 7.4 -20.0 10.0c-2.7 0.7 -3.0 0.8 -7.1 0.8c-4 0 -4.4-0.0 -7.1-0.7c-4.3-1.2 -9.7-3.6 -17.8-8.0c-2.3-1.2 -5.4-2.9 -6.8-3.7c-1.4-0.7 -3.9-2.1 -5.6-3.1c-1.6-0.9 -4.5-2.6 -6.5-3.6c-5.7-3.1 -16.8-10.0 -23.9-14.8c-1.2-0.8 -4.0-2.6 -6.4-4.1c-4.7-3.0 -11.2-7.5 -17.7-12.2c-2.3-1.7 -5.2-3.7 -6.2-4.5c-3.4-2.4 -9.3-7 -12.0-9.2c-1.4-1.2 -4.5-3.7 -6.7-5.7c-2.2-1.9 -5.5-4.5 -7.1-5.9c-6.0-5.0 -15.9-15.2 -21.2-21.8c-8.3-10.4 -11.9-16.8 -16.7-29.6c-2.4-6.5 -4.3-14.3 -6-25.5c-0.4-2.9 -0.9-11.6 -1.0-19.7c-0.1-7.9 0.6-21.4 1.5-28.3c0.3-2.5 0.8-5.9 1.0-7.5c0.2-1.5 0.5-3.5 0.7-4.2c0.1-0.7 0.4-2.2 0.6-3.3c0.1-1.1 0.6-3.1 0.9-4.6c0.3-1.4 0.8-3.3 1.1-4.2c1.0-4.1 2.5-8.7 4.7-14.6c1.9-5.0 6.9-13.6 10.8-18.4c3.9-4.8 9.9-9.5 15.6-12.0c4.9-2.2 13.3-4.3 24.6-6.3c6.6-1.1 10.5-1.6 15.1-1.8c5.3-0.2 20.8-0.3 25-0.0zm244.0 17.4c0.5 0.3 0.8 1.2 0.8 2.3c0 0.6 0.2 2.1 0.6 3.4c0.3 1.3 0.9 3.8 1.2 5.6c2.4 12.7 4.2 18.5 7.3 24.9c3.3 6.5 6.5 10.1 14.2 16.2c4.0 3.1 15.8 9.6 19.8 10.9c2.0 0.6 2.9 1.3 2.9 2.1c0 0.8 -0.9 1.3 -2.5 1.5c-1.6 0.1 -7.4 1.3 -12.1 2.5c-15.1 3.7 -25.1 10.1 -32.3 20.5c-2.5 3.7 -8.6 15.3 -11.0 21c-0.4 1.0 -1.4 3.1 -2.2 4.6c-1.0 2.0 -1.5 2.7 -1.9 2.8c-1.1 0.1 -1.6-1.6 -3.3-11.4c-1.4-8.4 -2.1-11.4 -4-16.5c-3.6-9.8 -5.9-13.9 -11.2-19.3c-5.4-5.5 -9.3-8.1 -20.6-13.9c-4.2-2.1 -6.3-3.1 -9.5-4.3c-1.4-0.5 -2.1-1.3 -1.9-2.2c0.2-0.8 1.1-1.2 3.6-1.4c7.6-0.8 19.3-3.6 26.1-6.3c5.3-2.0 7.2-3.1 10.7-5.8c6.2-4.9 11.5-12.1 16.6-22.4c2.8-5.7 6.2-12.9 6.4-13.5c0.1-0.6 1.2-1.5 1.7-1.5c0.1 0 0.4 0.1 0.5 0.2zm230.0 40.8c0.1 0.2 0.2 1.3 0.3 2.2c0.1 0.9 0.4 2.6 0.6 3.6c0.2 1.0 0.8 3.9 1.2 6.5c0.7 4.6 1.0 6.0 2.4 10.7c1.0 3.5 3.7 10.7 5.1 13.5c2.3 4.8 6.2 9.7 10.2 13.1c3.1 2.6 8.4 6.2 12 8.1c9.8 5.3 9.9 5.4 9.8 6.3c-0.1 1.2 -0.9 1.5 -6.2 2.2c-9.8 1.2 -19.7 4.7 -26.5 9.3c-6.6 4.5 -12.4 12.2 -17.5 23.1c-3.2 6.9 -6.6 13.4 -7.2 14.0c-0.5 0.4 -0.7 0.5 -1.1 0.2c-0.5-0.3 -0.9-1.2 -0.9-2.1c0-0.3 -0.2-1.8 -0.6-3.3c-0.7-3.3 -2.0-9.9 -2.2-11.6c-0.2-1.8 -2.2-9.5 -3.3-12.4c-1.1-3.3 -3.9-9.0 -5.6-11.6c-3.5-5.3 -9.2-10.5 -15.9-14.4c-1.0-0.6 -2.2-1.3 -2.7-1.6c-1.1-0.7 -8.4-4.2 -9.2-4.5c-1.5-0.4 -3.5-1.7 -3.6-2.3c-0.3-1.2 0.4-1.5 5.9-2.5c9.4-1.8 12.5-2.4 15.4-3.3c8.2-2.4 12.6-4.6 16.8-8.3c3.4-2.9 8.7-10.1 11.9-16.1c1.7-3.3 6.9-14.8 7.7-17.2c0.5-1.6 1-2.2 1.7-2.5c0.5-0.1 1.2 0.2 1.4 0.8z"/></svg>'
)
_HELMET_ICON_DATA_URI = "data:image/png;base64,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"

# Website palette: Charcoal Surface canvas (#1E1E1D), Rose Alloy text, soft Mist Green secondary text (no pure black or white).
_LOOPBACK_PAGE = """<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Factr-I</title>
<link rel="icon" type="image/png" href="{icon_uri}">
<style>
html,body{{height:100%;margin:0}}
body{{display:grid;place-items:center;background:#1E1E1D;color:#FFFDFC;
font:16px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif}}
main{{text-align:center;padding:0 24px}}
.mark{{display:block;width:56px;height:62px;margin:0 auto 28px}}
.mark svg{{display:block;width:100%;height:100%}}
h1{{margin:0 0 10px;font-size:28px;font-weight:600;letter-spacing:-0.01em}}
p{{margin:0;color:#BCCBC1}}
</style></head><body><main><span class="mark" role="img" aria-label="Factr-I">{icon_svg}</span><h1>{title}</h1><p>{sub}</p></main></body></html>"""


def _make_loopback_callback_handler(
    expected_path: str, *, display_name: str,
) -> tuple[type[BaseHTTPRequestHandler], dict[str, Any]]:
    """Handler class for an RFC 8252 loopback redirect plus the dict it fills in.

    Only a GET on *expected_path* is accepted (anything else is a 404 and leaves the result
    untouched), so a nonce embedded in the path acts as the CSRF ``state`` for authorization
    servers that do not echo an explicit ``state`` parameter.
    """
    result: dict[str, Any] = {"code": None, "state": None, "error": None, "error_description": None}

    class _LoopbackCallbackHandler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            parsed = urlparse(self.path)
            if parsed.path != expected_path:
                self.send_response(404)
                self.end_headers()
                self.wfile.write(b"Not found.")
                return

            params = parse_qs(parsed.query)
            for key in result:
                result[key] = params.get(key, [None])[0]

            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            failed = bool(result["error"])
            head = f"{display_name} sign-in didn't finish" if failed else "You're signed in"
            sub = "Return to Factr-I and try again." if failed else "You can close this tab and go back to Factr-I."
            self.wfile.write(_LOOPBACK_PAGE.format(title=head, sub=sub, icon_svg=_HELMET_ICON_SVG, icon_uri=_HELMET_ICON_DATA_URI).encode("utf-8"))

        def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
            return

    return _LoopbackCallbackHandler, result


def _bind_loopback_callback_server(
    host: str, port: int, handler_cls: type[BaseHTTPRequestHandler], *, err: Callable[..., AuthError],
    bind_failed_code: str,
) -> HTTPServer:
    """Bind the loopback listener up front (``port=0`` = OS-assigned) so the redirect URI sent to
    the authorization server names a port we already own — no probe-close-rebind race."""

    class _ReuseHTTPServer(HTTPServer):
        allow_reuse_address = True

    try:
        return _ReuseHTTPServer((host, port), handler_cls)
    except OSError as exc:
        raise err(f"Could not bind callback server on {host}:{port}: {exc}", bind_failed_code) from exc


def _serve_loopback_callback(
    server: HTTPServer, result: dict[str, Any], *, timeout_seconds: float, err: Callable[..., AuthError],
    timeout_code: str,
) -> dict[str, Any]:
    """Serve *server* until the redirect lands in *result* or the deadline passes; always closes."""
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True)
    thread.start()
    deadline = time.monotonic() + max(5.0, timeout_seconds)
    try:
        while time.monotonic() < deadline:
            if result["code"] or result["error"]:
                return result
            time.sleep(0.1)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=1.0)
    raise err("Authorization timed out waiting for the local callback.", timeout_code)


def _print_loopback_ssh_hint(redirect_uri: str, *, docs_url: str | None = None) -> None:
    """Print an SSH tunnel hint when a loopback-redirect OAuth flow runs on a remote host.

    The auth server redirects the browser to ``127.0.0.1:<port>/callback``; when the browser is
    on another machine (the SSH case) the redirect needs a local port forward to reach us.
    """
    from factr_backend.auth import _is_remote_session
    if not _is_remote_session():
        return
    try:
        parsed = urlparse(redirect_uri)
    except Exception:
        return
    host, port = parsed.hostname or "", parsed.port
    if host not in {"127.0.0.1", "::1", "localhost"} or not port:
        return
    divider = "-" * 60
    print(
        f"\n{divider}\nRemote session detected — SSH tunnel required\n{divider}\n"
        f"Factr is waiting for the OAuth callback on {redirect_uri}\n"
        "but your browser is on a different machine. Run this command\n"
        "in a NEW terminal on your local machine BEFORE opening the URL:\n\n"
        f"  ssh -N -L {port}:127.0.0.1:{port} {_ssh_user_at_host()}\n\n"
        "Then open the authorize URL above in your local browser.")
    if docs_url:
        print(f"Provider docs:      {docs_url}")
    print(f"SSH/jump-box guide: {OAUTH_OVER_SSH_DOCS_URL}\n{divider}\n")


def _default_verify() -> bool | ssl.SSLContext:
    """Platform-aware default SSL verify for httpx clients.

    On macOS with Homebrew Python the system OpenSSL cannot find the system trust store, so pin
    certifi's bundle when importable; elsewhere defer to httpx's built-in default.
    """
    if sys.platform == "darwin":
        try:
            import certifi
            return ssl.create_default_context(cafile=certifi.where())
        except ImportError:
            pass
    return True


def _resolve_verify(
    *, insecure: Optional[bool] = None, ca_bundle: Optional[str] = None,
    auth_state: Optional[Dict[str, Any]] = None) -> bool | ssl.SSLContext:
    from factr_backend.auth import _default_verify
    tls_state = auth_state.get("tls") if isinstance(auth_state, dict) else {}
    tls_state = tls_state if isinstance(tls_state, dict) else {}
    effective_insecure = (
        is_truthy_value(insecure, default=False) if insecure is not None
        else is_truthy_value(tls_state.get("insecure", False), default=False))
    effective_ca = (
        ca_bundle or tls_state.get("ca_bundle") or os.getenv("FACTR_CA_BUNDLE")
        or os.getenv("SSL_CERT_FILE") or os.getenv("REQUESTS_CA_BUNDLE"))
    if effective_insecure:
        return False
    if effective_ca:
        ca_path = str(effective_ca)
        if not os.path.isfile(ca_path):
            logger.warning(
                "CA bundle path does not exist: %s — falling back to default certificates",
                ca_path)
            return _default_verify()
        return ssl.create_default_context(cafile=ca_path)
    return _default_verify()


def _print_device_code_instructions(
    verification_url: str, user_code: str, *, open_browser: bool, failure_dash: str = "--",
    swallow_open_errors: bool = False) -> None:
    """Print the shared "To continue" device-code block and optionally open the browser.

    Callers decide *whether* to open (remote-session / graphical-browser gating differs per
    provider); *failure_dash* keeps each provider's historical hint wording.
    """
    print()
    print("To continue:")
    print(f"  1. Open: {verification_url}")
    print(f"  2. If prompted, enter code: {user_code}")
    if not open_browser:
        return
    try:
        opened = webbrowser.open(verification_url)
    except Exception:
        if not swallow_open_errors:
            raise
        opened = False
    if opened:
        print("  (Opened browser for verification)")
    else:
        print(f"  Could not open browser automatically {failure_dash} use the URL above.")


def _poll_device_token_generic(
    post: Callable[[], "httpx.Response"], *, expires_in: int, poll_interval: int,
    validate_success: Callable[[Dict[str, Any]], None],
    on_non_json_error: Callable[["httpx.Response"], Exception],
    on_error: Callable[["httpx.Response", Dict[str, Any]], Exception],
    on_timeout: Callable[[], Exception]) -> Dict[str, Any]:
    """RFC 8628 device-code polling loop shared by the device-code flows (xAI).

    ``authorization_pending`` sleeps and retries; ``slow_down`` grows the interval by 1s (cap 30s).
    Every other error, a non-JSON error body, and the deadline become provider-specific exceptions
    via the supplied factories so each caller keeps its exact error contract.
    """
    deadline = time.monotonic() + max(1, expires_in)
    current_interval = poll_interval
    while time.monotonic() < deadline:
        response = post()
        if response.status_code == 200:
            payload = response.json()
            validate_success(payload)
            return payload
        try:
            error_payload = response.json()
        except Exception:
            response.raise_for_status()
            raise on_non_json_error(response)
        error_code = str(error_payload.get("error") or "")
        if error_code == "authorization_pending":
            time.sleep(current_interval)
            continue
        if error_code == "slow_down":
            current_interval = min(current_interval + 1, 30)
            time.sleep(current_interval)
            continue
        raise on_error(response, error_payload)
    raise on_timeout()


def _prompt_yes_no(prompt: str, *, default: str) -> bool:
    """``input()`` a [Y/n]-style question; EOF/Ctrl-C count as *default*."""
    try:
        answer = input(prompt).strip().lower()
    except (EOFError, KeyboardInterrupt):
        answer = default
    return answer in {"", "y", "yes"} if default == "y" else answer in {"y", "yes"}


def _print_login_success(
    provider_id: str, config_path: Path, *, show_auth_state: bool = False) -> None:
    print()
    print("Login successful!")
    if show_auth_state:
        from factr_constants import display_factr_home as _dhh
        print(f"  Auth state: {_dhh()}/auth.json")
    print(f"  Config updated: {config_path} (model.provider={provider_id})")


def _offer_existing_oauth_credentials(
    provider_id: str, *, resolve: Callable[[], Dict[str, Any]],
    is_expiring: Callable[[str, int], bool], display_name: str, default_base_url: str,
    expired_notice: Optional[str] = None) -> bool:
    """Offer to reuse still-valid stored OAuth credentials. Returns True when the user accepted.

    *resolve* attempts a refresh, so a resolved token should be valid — but double-check the
    expiry before telling the user "Login successful!".
    """
    from factr_backend.auth import _update_config_for_provider
    try:
        existing = resolve()
        api_key = existing.get("api_key", "")
        if isinstance(api_key, str) and api_key and not is_expiring(api_key, 60):
            print(f"Existing {display_name} credentials found in Factr auth store.")
            if _prompt_yes_no("Use existing credentials? [Y/n]: ", default="y"):
                config_path = _update_config_for_provider(
                    provider_id, existing.get("base_url", default_base_url))
                _print_login_success(provider_id, config_path)
                return True
        elif expired_notice:
            print(expired_notice)
    except AuthError:
        pass
    return False
