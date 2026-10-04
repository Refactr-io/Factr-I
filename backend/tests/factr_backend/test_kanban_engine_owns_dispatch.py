"""Under the Factr-I engine the default kanban worker (a second Factr-I runtime) must not spawn."""
from factr_backend import kanban_db as kb
from factr_backend import kanban_db_dispatch as kbd


def _board(tmp_path, monkeypatch):
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    kb.init_db()
    conn = kb.connect()
    tid = kb.create_task(conn, title="t", assignee="default")
    return conn, tid


def test_default_spawn_refused_when_engine_set(tmp_path, monkeypatch):
    conn, tid = _board(tmp_path, monkeypatch)
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    monkeypatch.setattr(kbd, "_default_spawn", lambda *a, **k: (_ for _ in ()).throw(AssertionError("spawned")))
    res = kbd.dispatch_once(conn)
    assert not res.spawned
    assert kb.get_task(conn, tid).status == "ready"


def test_explicit_spawn_fn_still_runs_when_engine_set(tmp_path, monkeypatch):
    conn, tid = _board(tmp_path, monkeypatch)
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1")
    res = kbd.dispatch_once(conn, spawn_fn=lambda task, ws, board=None: 1)
    assert [s[0] for s in res.spawned] == [tid]
