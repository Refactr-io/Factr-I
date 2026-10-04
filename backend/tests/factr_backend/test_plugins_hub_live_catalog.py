"""The plugins hub and ``factr plugins list`` resolve the bundled kill list without any network I/O."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from factr_backend import plugin_catalog as pc
from factr_backend import plugins_cmd
from factr_backend import web_server
import factr_backend.config as _cfg_mod
import factr_backend.web_server_dashboard as _web_server_dashboard
import factr_backend.web_server_memory as _web_server_memory
from tools import registry as tools_registry


@pytest.fixture(autouse=True)
def _isolated_live_catalog():
    tools_registry.invalidate_check_fn_cache()
    _web_server_dashboard._invalidate_plugins_hub_cache()


class _UnreachableCatalog:
    """Counts network attempts; every one fails like a dead host."""

    def __init__(self):
        self.attempts = 0

    def __call__(self, *args, **kwargs):
        self.attempts += 1
        raise OSError("catalog host unreachable")


_PLUGIN_ROWS = [
    ("demo", "1.0.0", "demo plugin", "user", "/tmp/demo-plugin", "demo"),
    ("second", "0.2.0", "second plugin", "user", "/tmp/second-plugin", "second"),
    ("third", "0.3.0", "third plugin", "user", "/tmp/third-plugin", "third"),
]


def test_hub_rebuild_and_plugins_list_resolve_the_kill_list_once(monkeypatch, tmp_path, capsys):
    """Both listing surfaces make no network attempt and still report an in-tree removal."""
    unreachable = _UnreachableCatalog()
    monkeypatch.setattr("httpx.get", unreachable)
    monkeypatch.setattr(pc, "get_catalog_dir", lambda: tmp_path)
    (tmp_path / "removed.yaml").write_text("removed:\n- name: demo\n  reason: exfiltrated env vars\n")

    monkeypatch.setattr(web_server, "_get_dashboard_plugins", lambda force_rescan=False: [])
    monkeypatch.setattr(_web_server_memory, "_discover_memory_provider_statuses", lambda: [])
    monkeypatch.setattr(_cfg_mod, "get_factr_home", lambda: Path("/tmp/factr-home"))
    monkeypatch.setattr(_cfg_mod, "load_config", lambda: {"dashboard": {"hidden_plugins": []}})
    monkeypatch.setattr(plugins_cmd, "_discover_all_plugins", lambda: list(_PLUGIN_ROWS))
    monkeypatch.setattr(plugins_cmd, "_get_current_context_engine", lambda: "compressor")
    monkeypatch.setattr(plugins_cmd, "_get_current_memory_provider", lambda: "")
    monkeypatch.setattr(plugins_cmd, "_discover_context_engines", lambda: [])
    monkeypatch.setattr(plugins_cmd, "_get_disabled_set", lambda: set())
    monkeypatch.setattr(plugins_cmd, "_get_enabled_set", lambda: {"demo"})
    monkeypatch.setattr(plugins_cmd, "_read_manifest", lambda _path: {"provides_tools": []})
    monkeypatch.setattr(plugins_cmd, "_read_install_metadata", lambda: {})
    monkeypatch.setattr(tools_registry.registry, "get_entry", lambda _name: SimpleNamespace(check_fn=None))

    payload = _web_server_dashboard._merged_plugins_hub(force_refresh=True)
    by_name = {row["name"]: row["removed_reason"] for row in payload["plugins"]}
    assert by_name == {"demo": "exfiltrated env vars", "second": None, "third": None}
    assert unreachable.attempts == 0

    plugins_cmd.cmd_list(argparse.Namespace(enabled=False, user=False, no_bundled=False, plain=False, json=True))
    rows = {row["name"]: row["removed"] for row in json.loads(capsys.readouterr().out)}
    assert rows == {"demo": "exfiltrated env vars", "second": None, "third": None}
    assert unreachable.attempts == 0
