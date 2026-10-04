"""Resolve FACTR_CONFIG_HOME for standalone skill scripts.

Skill scripts may run outside the Factr process (e.g. system Python,
nix env, CI) where ``factr_constants`` is not importable.  This module
provides the same ``get_factr_home()`` and ``display_factr_home()``
contracts as ``factr_constants`` without requiring it on ``sys.path``.

When ``factr_constants`` IS available it is used directly so that any
future enhancements (profile resolution, Docker detection, etc.) are
picked up automatically.  The fallback path replicates the core logic
from ``factr_constants.py`` using only the stdlib.

All scripts under ``google-workspace/scripts/`` should import from here
instead of duplicating the ``FACTR_CONFIG_HOME = Path(os.getenv(...))`` pattern.
"""

from __future__ import annotations

import os
from pathlib import Path

try:
    from factr_constants import display_factr_home as display_factr_home
    from factr_constants import get_factr_home as get_factr_home
except (ModuleNotFoundError, ImportError):

    def get_factr_home() -> Path:
        """Return the Factr home directory (default: ~/.factr).

        Mirrors ``factr_constants.get_factr_home()``."""
        val = os.environ.get("FACTR_CONFIG_HOME", "").strip()
        return Path(val) if val else Path.home() / ".factr"

    def display_factr_home() -> str:
        """Return a user-friendly ``~/``-shortened display string.

        Mirrors ``factr_constants.display_factr_home()``."""
        home = get_factr_home()
        try:
            return "~/" + home.relative_to(Path.home()).as_posix()
        except ValueError:
            return str(home)
