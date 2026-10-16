"""Desktop dictation warm-up: the local model files are fetched in the background, never loaded."""
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
