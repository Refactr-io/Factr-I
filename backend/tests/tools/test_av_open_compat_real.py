"""The av-open compatibility shim against the REAL packages (skipped unless faster-whisper and av are installed).

Run it with the bundled runtime and a target holding faster-whisper 1.2.1 + a PyAV newer than 17 to prove the fix end to end:
    uv pip install --python <bundled python> --target /tmp/av19 faster-whisper==1.2.1 numpy==2.4.3 'av>=19'
    PYTHONPATH=/tmp/av19:backend python -m pytest tests/tools/test_av_open_compat_real.py
"""
import math
import struct
import wave

import pytest

av = pytest.importorskip("av")
pytest.importorskip("faster_whisper")


@pytest.fixture()
def tone(tmp_path):
    path = tmp_path / "tone.wav"
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * 440 * i / 16000))) for i in range(16000)))
    return str(path)


def test_faster_whisper_decodes_with_the_installed_av(tone):
    from faster_whisper.audio import decode_audio

    from tools.transcription_local import _ensure_av_open_compat

    _ensure_av_open_compat()
    audio = decode_audio(tone, sampling_rate=16000)
    assert audio.shape == (16000,)


def _major(version: str) -> int:
    return int(version.split(".")[0])


@pytest.mark.skipif(_major(av.__version__) < 18, reason="needs the incompatible PyAV (18+) this regression is about")
def test_the_incompatible_pair_fails_without_the_shim_and_decodes_with_it(tone):
    """The premise, asserted: faster-whisper 1.2.1 + PyAV >= 18 raises on the removed keyword; the shim is what fixes it."""
    from faster_whisper.audio import decode_audio

    from tools import transcription_local

    original = av.open
    if getattr(original, "_factr_compat", False):  # an earlier test in this process already installed the shim
        av.open = getattr(original, "_orig", original)
    try:
        with pytest.raises(TypeError, match="metadata_errors"):
            decode_audio(tone, sampling_rate=16000)
    finally:
        av.open = original
    transcription_local._ensure_av_open_compat()
    assert decode_audio(tone, sampling_rate=16000).shape == (16000,)
