"""The per-call log line carries what a cost or cache investigation needs: the cache write count, the
provider's response id, and the serving upstream when the route reports one. Existing parsers read a
prefix of the line, so the fields are appended and optional."""
import logging
from types import SimpleNamespace


def _agent(tmp_path, monkeypatch):
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    from run_agent import AIAgent
    return AIAgent(api_key="k", base_url="https://openrouter.ai/api/v1", provider="openrouter",
                   api_mode="chat_completions", model="anthropic/claude-fable-5.1", session_id="t", platform="cli",
                   quiet_mode=True, skip_context_files=True, skip_memory=True, save_trajectories=False, enabled_toolsets=["file"])


def _usage(read, write, prompt):
    return SimpleNamespace(prompt_tokens=prompt, completion_tokens=7, total_tokens=prompt + 7,
                           prompt_tokens_details=SimpleNamespace(cached_tokens=read, cache_write_tokens=write),
                           completion_tokens_details=None)


def _line(agent, caplog, resp):
    from agent import turn_usage
    caplog.clear()
    with caplog.at_level(logging.INFO, logger="agent.turn_usage"):
        turn_usage.record_response_usage(agent, resp, messages=[{"role": "user", "content": "hi"}], api_call_count=1,
                                         api_duration=0.2, compression_attempts=0, max_compression_attempts=3)
    return next(r.getMessage() for r in caplog.records if r.getMessage().startswith("API call #"))


def test_write_id_and_upstream_are_on_the_line(tmp_path, monkeypatch, caplog):
    a = _agent(tmp_path, monkeypatch)
    try:
        line = _line(a, caplog, SimpleNamespace(usage=_usage(34_283, 28_604, 62_889), id="gen-1788636728-qMa1SbYZcwjrvUzJuF1g",
                                                provider="Anthropic", model="anthropic/claude-fable-5.1"))
    finally:
        a.close()
    assert " cache=34283/62889 (55%)" in line
    assert " write=28604" in line
    assert " id=gen-1788636728-qMa1SbYZcwjrvUzJuF1g" in line
    assert " upstream=Anthropic" in line
    # the pre-existing prefix is unchanged, so older parsers keep matching
    assert line.startswith("API call #1: model=anthropic/claude-fable-5.1 provider=openrouter in=62889 out=7 total=62896 latency=0.2s")


def test_fields_are_omitted_when_absent(tmp_path, monkeypatch, caplog):
    a = _agent(tmp_path, monkeypatch)
    try:
        line = _line(a, caplog, SimpleNamespace(usage=_usage(0, 0, 100), id=None, model="anthropic/claude-fable-5.1"))
    finally:
        a.close()
    assert "write=" not in line and "id=" not in line and "upstream=" not in line
