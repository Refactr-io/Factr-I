"""Tests for the top-level `./factr-backend` launcher script."""

import runpy
import sys
import types
from pathlib import Path


def test_launcher_delegates_to_argparse_entrypoint(monkeypatch):
    """`./factr-backend` should use `factr_backend.main`, not the legacy Fire wrapper."""
    launcher_path = Path(__file__).resolve().parents[2] / "factr-backend"
    called = []

    fake_main_module = types.ModuleType("factr_backend.main")

    def fake_main():
        called.append("factr_backend.main")

    fake_main_module.main = fake_main
    monkeypatch.setitem(sys.modules, "factr_backend.main", fake_main_module)

    fake_cli_module = types.ModuleType("cli")

    def legacy_cli_main(*args, **kwargs):
        raise AssertionError("launcher should not import cli.main")

    fake_cli_module.main = legacy_cli_main
    monkeypatch.setitem(sys.modules, "cli", fake_cli_module)

    fake_fire_module = types.ModuleType("fire")

    def legacy_fire(*args, **kwargs):
        raise AssertionError("launcher should not invoke fire.Fire")

    fake_fire_module.Fire = legacy_fire
    monkeypatch.setitem(sys.modules, "fire", fake_fire_module)

    monkeypatch.setattr(sys, "argv", [str(launcher_path), "gateway", "status"])

    runpy.run_path(str(launcher_path), run_name="__main__")

    assert called == ["factr_backend.main"]
