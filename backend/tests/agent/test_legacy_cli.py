"""The packaged ``factr-backend`` console script honours argv (#54648).

A console script calls its target with no arguments; these tests go through the
target named in pyproject ``[project.scripts]`` exactly the way pip's wrapper does.
"""

from __future__ import annotations

import importlib
import sys
import tomllib
from pathlib import Path

import pytest

import run_agent


def _run_console_script(monkeypatch, *argv: str):
    pyproject = Path(__file__).resolve().parents[2] / "pyproject.toml"
    module, func = tomllib.loads(pyproject.read_text(encoding="utf-8"))["project"]["scripts"]["factr-backend"].split(":")
    monkeypatch.setattr(sys, "argv", ["factr-backend", *argv])
    try:
        return getattr(importlib.import_module(module), func)()
    except SystemExit as exc:
        return exc.code


@pytest.mark.parametrize("argv", [("--help",), ("-h",), ("--version",), ()])
def test_metadata_invocations_never_start_an_agent(argv, monkeypatch, capsys):
    def _no_agent(**_kwargs):
        raise AssertionError("a metadata invocation built an agent")

    monkeypatch.setattr(run_agent, "AIAgent", _no_agent)

    assert _run_console_script(monkeypatch, *argv) in (0, None)
    out = capsys.readouterr().out
    assert "usage: factr" in out or "Factr-I v" in out
