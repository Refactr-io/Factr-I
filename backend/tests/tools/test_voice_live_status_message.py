"""GPT-Live needs an OpenAI API key; a ChatGPT/Codex login is not one, and the greyed-out menu item must say so."""
import tools.voice_live as vl


def test_status_without_a_key_explains_that_a_chatgpt_login_is_not_enough(monkeypatch):
    monkeypatch.setattr(vl, "_resolve_credentials", lambda live: (None, "https://api.openai.com/v1"))
    status = vl.resolve_gpt_live_status()
    assert status["available"] is False
    assert "API key" in status["reason"]
    assert "ChatGPT or Codex login is not enough" in status["reason"]


def test_status_with_a_key_has_no_reason(monkeypatch):
    monkeypatch.setattr(vl, "_resolve_credentials", lambda live: ("sk-test", "https://api.openai.com/v1"))
    status = vl.resolve_gpt_live_status()
    assert status["available"] is True and status["reason"] is None


def test_creating_a_session_without_a_key_says_the_same(monkeypatch):
    monkeypatch.setattr(vl, "_resolve_credentials", lambda live: (None, "https://api.openai.com/v1"))
    try:
        vl.create_webrtc_session("v=0\r\n")
    except ValueError as exc:
        assert "ChatGPT or Codex login is not enough" in str(exc)
    else:
        raise AssertionError("expected ValueError")
