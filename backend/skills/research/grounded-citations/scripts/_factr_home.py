"""Resolve FACTR_CONFIG_HOME for standalone skill scripts.

Skill scripts may run outside the Factr process (system Python, nix env,
CI) where ``factr_constants`` is not importable.  This module provides the
same ``get_factr_home()`` contract without requiring it on ``sys.path``.

When ``factr_constants`` IS available it is used directly so profile
resolution and any future enhancements are picked up automatically.
"""

from __future__ import annotations

import os
from pathlib import Path

try:
    from factr_constants import get_factr_home as get_factr_home
except (ModuleNotFoundError, ImportError):

    def get_factr_home() -> Path:
        """Return the Factr home directory (default: ``~/.factr``)."""
        val = os.environ.get("FACTR_CONFIG_HOME", "").strip()
        return Path(val) if val else Path.home() / ".factr"
