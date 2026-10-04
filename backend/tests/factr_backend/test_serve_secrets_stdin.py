"""`serve --secrets-stdin` takes the session and engine tokens from stdin, keeping them out of the launch env."""

import io

import pytest

from factr_backend.main_dashboard import _adopt_stdin_secrets
from factr_backend.subcommands.dashboard import build_serve_parser


def test_tokens_come_from_stdin(monkeypatch):
    for k in ("FACTR_DASHBOARD_SESSION_TOKEN", "FACTR_ENGINE_TOKEN"):
        monkeypatch.delenv(k, raising=False)
    _adopt_stdin_secrets(io.StringIO('{"session_token": "sess", "engine_token": "eng"}\n'))
    import os

    assert (os.environ["FACTR_DASHBOARD_SESSION_TOKEN"], os.environ["FACTR_ENGINE_TOKEN"]) == ("sess", "eng")


def test_bad_stdin_fails_closed():
    with pytest.raises(SystemExit):
        _adopt_stdin_secrets(io.StringIO("\n"))


def test_flag_is_opt_in():
    parse = build_serve_parser(cmd_dashboard=lambda a: None).parse_args
    assert parse([]).secrets_stdin is False
    assert parse(["--secrets-stdin"]).secrets_stdin is True
