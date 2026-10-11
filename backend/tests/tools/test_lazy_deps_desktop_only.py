"""The desktop app lets only voice transcription install packages at runtime."""
from tools import lazy_deps


def test_only_listed_features_may_install(monkeypatch):
    monkeypatch.setenv("FACTR_LAZY_INSTALL_ONLY", "stt.faster_whisper")
    assert lazy_deps._unsupported_feature_reason("stt.faster_whisper") is None
    for feature in ("provider.bedrock", "provider.vertex", "stt.mistral"):
        reason = lazy_deps._unsupported_feature_reason(feature)
        assert reason and reason.startswith("unsupported ")


def test_prefix_entry_covers_the_namespace(monkeypatch):
    monkeypatch.setenv("FACTR_LAZY_INSTALL_ONLY", "stt")
    assert lazy_deps._unsupported_feature_reason("stt.mistral") is None
    assert lazy_deps._unsupported_feature_reason("provider.bedrock")


def test_unset_keeps_every_feature(monkeypatch):
    monkeypatch.delenv("FACTR_LAZY_INSTALL_ONLY", raising=False)
    assert lazy_deps._unsupported_feature_reason("provider.bedrock") is None
