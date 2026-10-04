"""execute_code child env honors the multiplexed per-turn FACTR_CONFIG_HOME override (#110303).

Under a multiplexed Desktop/Dashboard connection one server process serves several
profiles, binding a context-local FACTR_CONFIG_HOME override per turn. ``_build_child_env``
scrubs the server process's ``os.environ`` — which carries the machine-default
FACTR_CONFIG_HOME — so without the rewrite below, skill scripts run via ``execute_code``
silently read/write the wrong profile's directory.
"""

import sys

import pytest

from factr_constants import (
    get_factr_home_override,
    reset_factr_home_override,
    set_factr_home_override,
)
from tools.code_execution_env import _build_child_env


@pytest.fixture
def home_override():
    tokens = []

    def _set(path):
        tokens.append(set_factr_home_override(path))
        return str(path)

    yield _set
    for token in reversed(tokens):
        reset_factr_home_override(token)


def _child_env():
    return _build_child_env(
        rpc_endpoint="sock",
        rpc_token="tok",
        tmpdir="/tmp/factr-test",
        child_python=sys.executable,
    )


class TestMultiplexedFactrHome:
    def test_override_rewrites_stale_server_default_per_turn(self, monkeypatch, home_override, tmp_path):
        """The reported bug: a child must see the ACTIVE profile's home, not the server default,
        and sequential turns for different profiles each see their own."""
        monkeypatch.setenv("FACTR_CONFIG_HOME", "/machine/default/.factr")
        alpha = home_override(tmp_path / "profiles" / "alpha")
        assert _child_env()["FACTR_CONFIG_HOME"] == alpha
        beta = home_override(tmp_path / "profiles" / "beta")
        assert _child_env()["FACTR_CONFIG_HOME"] == beta

    def test_no_override_leaves_inherited_value_untouched(self, monkeypatch):
        """Dedicated per-profile processes (no override): zero behavior change."""
        assert get_factr_home_override() is None
        monkeypatch.setenv("FACTR_CONFIG_HOME", "/machine/default/.factr")

        assert _child_env()["FACTR_CONFIG_HOME"] == "/machine/default/.factr"
