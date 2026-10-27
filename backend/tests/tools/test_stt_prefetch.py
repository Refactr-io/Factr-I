"""Local dictation preparation: started by the first use of dictation (never at startup), with a visible state."""
import sys
import types

import tools.transcription_local as local
import tools.transcription_tools as tt


def _fake_fw(monkeypatch, download):
    utils = types.ModuleType("faster_whisper.utils")
    utils.download_model = download
    monkeypatch.setitem(sys.modules, "faster_whisper", types.ModuleType("faster_whisper"))
    monkeypatch.setitem(sys.modules, "faster_whisper.utils", utils)


def test_prefetch_downloads_the_configured_model(monkeypatch):
    got = []
    _fake_fw(monkeypatch, lambda name: got.append(name) or "/cache")
    assert local.prefetch_local_stt_model("small") is True
    assert got == ["small"]


def test_prefetch_swallows_network_failure(monkeypatch):
    def boom(name):
        raise OSError("offline")
    _fake_fw(monkeypatch, boom)
    assert local.prefetch_local_stt_model() is False


def test_background_prefetch_skips_cloud_provider(monkeypatch):
    calls = []
    monkeypatch.setattr(tt, "_load_stt_config", lambda: {"enabled": True, "provider": "openai"})
    monkeypatch.setattr(local, "prefetch_local_stt_model", lambda *a: calls.append(a))
    monkeypatch.setattr(tt.threading, "Thread", lambda target, **k: types.SimpleNamespace(start=target))
    tt.prefetch_stt_in_background()
    assert calls == []


def test_background_prefetch_runs_for_local(monkeypatch):
    calls = []
    monkeypatch.setattr(tt, "_load_stt_config", lambda: {"enabled": True, "provider": "local", "local": {"model": "base"}})
    monkeypatch.setattr(tt, "_HAS_FASTER_WHISPER", True)
    monkeypatch.setattr(tt, "_has_local_command", lambda: False)
    monkeypatch.setattr(local, "prefetch_local_stt_model", lambda *a: calls.append(a))
    monkeypatch.setattr(tt.threading, "Thread", lambda target, **k: types.SimpleNamespace(start=target))
    tt.prefetch_stt_in_background()
    assert calls == [("base",)]


def _patch_local(monkeypatch, **cfg):
    monkeypatch.setattr(tt, "_load_stt_config", lambda: {"enabled": True, "provider": "local", "local": {"model": "base"}, **cfg})
    monkeypatch.setattr(tt, "_has_local_command", lambda: False)
    monkeypatch.setattr(tt.threading, "Thread", lambda target, **k: types.SimpleNamespace(start=target))
    tt._set_prepare("idle")


def test_states_run_from_installing_to_ready(monkeypatch):
    _patch_local(monkeypatch)
    seen = []
    monkeypatch.setattr(tt, "_HAS_FASTER_WHISPER", False)
    monkeypatch.setattr(tt, "_try_lazy_install_stt", lambda: seen.append(tt.stt_prepare_status()["state"]) or True)
    monkeypatch.setattr(local, "prefetch_local_stt_model", lambda *a: seen.append(tt.stt_prepare_status()["state"]) or True)
    tt.prepare_stt_in_background()
    assert seen == ["installing", "downloading"]
    assert tt.stt_prepare_status()["state"] == "ready"


def test_a_failed_install_is_reported_and_can_be_retried(monkeypatch):
    _patch_local(monkeypatch)
    monkeypatch.setattr(tt, "_HAS_FASTER_WHISPER", False)
    monkeypatch.setattr(tt, "_try_lazy_install_stt", lambda: False)
    tt.prepare_stt_in_background()
    status = tt.stt_prepare_status()
    assert status["state"] == "failed" and "installed" in status["detail"]
    monkeypatch.setattr(tt, "_try_lazy_install_stt", lambda: True)
    monkeypatch.setattr(local, "prefetch_local_stt_model", lambda *a: True)
    tt.prepare_stt_in_background()
    assert tt.stt_prepare_status()["state"] == "ready"


def test_a_running_preparation_is_not_started_twice(monkeypatch):
    _patch_local(monkeypatch)
    started = []
    monkeypatch.setattr(tt.threading, "Thread", lambda target, **k: types.SimpleNamespace(start=lambda: started.append(1)))
    monkeypatch.setattr(tt, "_HAS_FASTER_WHISPER", True)
    tt.prepare_stt_in_background()
    tt.prepare_stt_in_background()
    assert started == [1]


def test_nothing_to_prepare_for_a_cloud_provider(monkeypatch):
    monkeypatch.setattr(tt, "_load_stt_config", lambda: {"enabled": True, "provider": "openai"})
    assert tt.stt_prepare_status()["state"] == "not_needed"


def test_a_fresh_desktop_start_does_not_prepare_dictation():
    """The startup path must not fetch anything: the fresh start has no installer process and no network."""
    import inspect
    import factr_backend.web_server as ws

    source = inspect.getsource(ws)
    assert "prefetch_stt_in_background" not in source and "prepare_stt_in_background" not in source
