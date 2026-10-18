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


def test_scheduled_cross_profile_reply_uses_the_courier_and_one_durable_id(monkeypatch, tmp_path):
    from cron.engine_bot_chat import deliver_other
    from tools import bot_relay
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    monkeypatch.setattr(bot_relay, "relay_alive", lambda root: True)
    real = bot_relay.enqueue_envelope
    calls = []
    def enqueue(root, **values):
        envelope = real(root, **values)
        calls.append(envelope)
        bot_relay.write_reply(root, envelope["id"], reply="processed")
        return envelope
    monkeypatch.setattr(bot_relay, "enqueue_envelope", enqueue)
    for _ in range(2):
        result = deliver_other("A scheduled result", "a" * 64, "b", tmp_path)
        assert result["ok"] and result["reply"] == "processed"
    assert len(calls) == 1
    assert calls[0]["id"] == "a" * 32
    assert calls[0]["target_profile"] == "b"
    assert calls[0]["message"] == "A scheduled result"


def test_reusing_an_envelope_for_another_payload_is_rejected(tmp_path):
    import pytest
    from tools.bot_relay import enqueue_envelope, local_target
    values = dict(target=local_target("b", "b"), sender_profile="a", sender_handle="a", envelope_id="b" * 32)
    first = enqueue_envelope(tmp_path, message="first", **values)
    assert enqueue_envelope(tmp_path, message="first", **values) == first
    with pytest.raises(ValueError, match="different payload"):
        enqueue_envelope(tmp_path, message="second", **values)
    with pytest.raises(ValueError, match="invalid envelope"):
        enqueue_envelope(tmp_path, message="bad", **{**values, "envelope_id": "../escape"})


def test_only_this_hosts_computer_roster_can_resolve_an_unmounted_delivery_profile(monkeypatch, tmp_path):
    from cron.scheduler_delivery import _resolve_bot_chat_target
    from tools.bot_relay import write_remote_roster
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    monkeypatch.setattr("factr_backend.profiles.profile_exists", lambda name: False)
    row = {"profile": "beta", "handle": "beta", "connection_id": "another-host"}
    write_remote_roster(tmp_path, [row])
    assert _resolve_bot_chat_target({"id": "test"}, "beta") is None
    write_remote_roster(tmp_path, [{**row, "connection_id": "computer-beta"}])
    assert _resolve_bot_chat_target({"id": "test"}, "beta")["chat_id"] == "beta"
