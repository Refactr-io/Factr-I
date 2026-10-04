"""The engine's BRIDGED_TOOLSETS list (engine/crates/factr-gateway/src/rpc/toolsets.rs) must equal the
backend toolsets whose tools only this backend provides, so a run that names none of them never sees the
`factr` bridge tool. Drift in either direction fails here."""
import re
from pathlib import Path

import pytest

import toolsets

SRC = Path(__file__).resolve().parents[2] / "engine/crates/factr-gateway/src/rpc/toolsets.rs"

# Backend toolsets that are not bridged features: no tools of their own (context/room hooks), a role-gated
# onboarding surface, web_search-only (the engine's own `web` mapping covers it), or a bundle/posture.
NOT_BRIDGED = {"search", "context_engine", "bot_room", "setup", "debugging", "safe", "coding"}


def _engine():
    if not SRC.exists():
        pytest.skip("engine source not in this checkout")
    src = SRC.read_text()
    bridged = set(re.findall(r'"([a-z_]+)"', re.search(r"BRIDGED_TOOLSETS: &\[&str\] = &\[(.*?)\];", src, re.S).group(1)))
    mapped = set(re.findall(r'^\s*"([a-z_]+)" =>', re.search(r"fn tools_of.*?\n}\n", src, re.S).group(0), re.M))
    return bridged, mapped


def test_engine_bridged_list_matches_backend_registry():
    bridged, mapped = _engine()
    backend = {
        n for n, spec in toolsets.TOOLSETS.items()
        if spec["tools"] and not spec["includes"] and not n.startswith("factr-")
    }
    expected = (backend - mapped - NOT_BRIDGED) | {"cronjob"}
    assert bridged == expected, f"engine-only: {sorted(bridged - expected)}; backend-only: {sorted(expected - bridged)}"


def test_engine_bridged_names_are_backend_toolsets():
    bridged, _ = _engine()
    assert all(toolsets.validate_toolset(n) for n in bridged)
