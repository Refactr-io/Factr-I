"""faster-whisper 1.2.1 passes av.open(metadata_errors=...); PyAV 19 removed it (dictation failed)."""
import sys
import types

import pytest

from tools.transcription_local import _ensure_av_open_compat


@pytest.fixture
def fake_av(monkeypatch):
    av = types.ModuleType("av")

    def _open(file, mode="r", format=None, options=None):  # PyAV 19 signature: no metadata_errors
        return ("opened", file)

    av.open = _open
    monkeypatch.setitem(sys.modules, "av", av)
    return av


def test_metadata_errors_is_dropped_when_av_rejects_it(fake_av):
    with pytest.raises(TypeError, match="metadata_errors"):
        fake_av.open("a.wav", metadata_errors="ignore")
    _ensure_av_open_compat()
    assert fake_av.open("a.wav", mode="r", metadata_errors="ignore") == ("opened", "a.wav")


def test_shim_is_idempotent_and_keeps_other_type_errors(fake_av):
    _ensure_av_open_compat()
    first = fake_av.open
    _ensure_av_open_compat()
    assert fake_av.open is first
    with pytest.raises(TypeError, match="bogus"):
        fake_av.open("a.wav", bogus=1)


def test_av_that_accepts_the_argument_is_untouched(monkeypatch):
    av = types.ModuleType("av")
    av.open = lambda file, metadata_errors="strict": metadata_errors
    monkeypatch.setitem(sys.modules, "av", av)
    _ensure_av_open_compat()
    assert av.open("x", metadata_errors="ignore") == "ignore"


def test_voice_extra_and_setup_hook_pin_av():
    import tomllib
    from pathlib import Path
    from factr_backend.tools_config_post_setup import _PIP_POST_SETUP_HOOKS

    voice = tomllib.loads((Path(__file__).parents[2] / "pyproject.toml").read_text())["project"]["optional-dependencies"]["voice"]
    assert "av==17.0.0" in voice
    assert "av==17.0.0" in _PIP_POST_SETUP_HOOKS["faster_whisper"]["args"]
    assert "-U" not in _PIP_POST_SETUP_HOOKS["faster_whisper"]["args"]
