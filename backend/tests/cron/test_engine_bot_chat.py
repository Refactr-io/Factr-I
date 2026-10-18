import json

from cron.engine_bot_chat import deliver


def test_owner_engine_delivery_uses_literal_text_and_durable_id(monkeypatch):
    monkeypatch.setenv("FACTR_ENGINE_URL", "http://127.0.0.1:1234")
    monkeypatch.setenv("FACTR_ENGINE_TOKEN", "isolated-test-token")
    class Reply:
        def __enter__(self): return self
        def __exit__(self, *_): pass
        def read(self): return b'{"ok":true,"reply":"done"}'
    def open_request(request, timeout):
        assert request.full_url == "http://127.0.0.1:1234/api/bot/deliver"
        assert request.method == "POST"
        assert request.get_header("Authorization") == "Bearer isolated-test-token"
        assert json.loads(request.data) == {"text": "Literal $(keep) `keep` 'keep'", "envelope_id": "run-1"}
        assert timeout == 1230
        return Reply()
    monkeypatch.setattr("urllib.request.urlopen", open_request)
    assert deliver("Literal $(keep) `keep` 'keep'", "run-1") == {"ok": True, "reply": "done"}
