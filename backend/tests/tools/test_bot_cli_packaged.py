"""Bot Chat delivery in the packaged desktop app.

The packaged app ships no ``factr`` script that can run ``chat`` (resources/factr is the Rust engine,
which refuses it), so a delivery runs the bundled interpreter: ``<python> -m factr_backend.main``.
The turn lock and the live-owner home lookup must recognise that argv shape too.
"""
import sys
from pathlib import Path

from tools import bot_mode_dm, bot_relay


def test_packaged_delivery_argv_runs_the_bundled_python(monkeypatch):
    monkeypatch.setenv("FACTR_BACKEND_PYTHON", "/app/runtime/bin/python3.12")
    # A user-installed engine on PATH must not win: it refuses `chat`.
    monkeypatch.setattr(bot_relay.shutil, "which", lambda name: "/usr/local/bin/factr")
    argv = bot_relay.local_delivery_command("ops", "/tmp/q.txt")
    assert argv[:5] == [sys.executable, "-m", "factr_backend.main", "-p", "ops"]
    assert argv[5] == "chat" and argv[-2:] == ["--query-file", "/tmp/q.txt"]
    assert "--create-if-missing" in argv and "-Q" in argv


def test_unpackaged_delivery_argv_keeps_the_factr_script(monkeypatch):
    monkeypatch.delenv("FACTR_BACKEND_PYTHON", raising=False)
    monkeypatch.setattr(bot_relay, "_factr_backend", lambda: "/venv/bin/factr")
    assert bot_relay.local_delivery_command("ops", "q")[:3] == ["/venv/bin/factr", "-p", "ops"]


def test_bot_cli_profile_reads_both_shapes():
    assert bot_relay.bot_cli_profile(["/v/bin/factr", "-p", "a", "chat"]) == "a"
    assert bot_relay.bot_cli_profile(["factr.exe", "-p", "b"]) == "b"
    assert bot_relay.bot_cli_profile(["py", "-m", "factr_backend.main", "-p", "c", "chat"]) == "c"
    assert bot_relay.bot_cli_profile(["py", "-m", "factr_backend.main", "chat"]) is None
    assert bot_relay.bot_cli_profile(["py", "-m", "other", "-p", "c"]) is None
    assert bot_relay.bot_cli_profile(["python3", "-p", "c"]) is None
    assert bot_relay.bot_cli_profile([]) is None


def test_turn_lock_and_live_home_apply_to_the_packaged_argv(monkeypatch, tmp_path):
    seen = []

    class _Lock:
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    monkeypatch.setattr(bot_relay, "acquire_turn_lock", lambda root, profile: seen.append(profile) or _Lock())
    argv = ["py", "-m", "factr_backend.main", "-p", "ops", "chat"]
    with bot_mode_dm._delivery_lock(argv, stdin_file=False):
        pass
    assert seen == ["ops"]
    with bot_mode_dm._delivery_lock(argv, stdin_file=True):
        pass
    assert seen == ["ops"]  # peer transports are locked by the remote gateway

    home = tmp_path / "ops"
    monkeypatch.setattr("tools.bot_mode_probe._roster", lambda root: [("ops", home)])
    assert bot_mode_dm._local_delivery_home(argv) == home
    assert bot_mode_dm._local_delivery_home(["py", "-m", "factr_backend.main", "-p", "x"]) is None
