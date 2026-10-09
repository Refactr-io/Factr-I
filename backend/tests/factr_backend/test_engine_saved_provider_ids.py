"""The engine saves the default model's provider in config.yaml under the runtime's ids (the ChatGPT
login as ``openai-codex``, an OpenAI key as ``openai-api``, Claude as ``anthropic``, Meta's as ``muse``).
Each must resolve here too, both named and as the saved provider behind ``auto``."""

from __future__ import annotations

import pytest

from factr_backend.auth import resolve_provider
from factr_constants import reset_factr_home_override, set_factr_home_override

ENGINE_SAVED = {"openai-codex": "openai-codex", "openai-api": "openai-api", "anthropic": "anthropic", "muse": "meta-ai"}


@pytest.mark.parametrize("saved", sorted(ENGINE_SAVED))
def test_engine_saved_provider_resolves(saved, tmp_path):
    assert resolve_provider(saved) == ENGINE_SAVED[saved]
    (tmp_path / "config.yaml").write_text(f"model:\n  default: some-model\n  provider: {saved}\n", encoding="utf-8")
    token = set_factr_home_override(tmp_path)
    try:
        assert resolve_provider("auto") == ENGINE_SAVED[saved]
    finally:
        reset_factr_home_override(token)
