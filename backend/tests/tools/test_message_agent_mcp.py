"""Engine-side teammate messaging: the stdio MCP server and the per-profile config that enables it."""
import json
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest
import yaml

from tools import bot_relay, message_agent_mcp as mcp


def _profile(root: Path, name: str, *, bot: bool = True) -> Path:
    d = root / "profiles" / name
    d.mkdir(parents=True)
    (d / "config.yaml").write_text("model:\n  default: x\n", encoding="utf-8")
    if bot:
        (d / "profile.yaml").write_text(yaml.safe_dump({"ui_meta": {"factr-bots": {"title": name.title()}}}), encoding="utf-8")
    return d


@pytest.fixture
def root(tmp_path, monkeypatch):
    (tmp_path / "config.yaml").write_text("model:\n  default: root\n", encoding="utf-8")
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    return tmp_path


def test_server_entry_lands_only_in_bot_profiles_never_default(root):
    a, b, plain = _profile(root, "alice"), _profile(root, "bob"), _profile(root, "plain", bot=False)
    default_before = (root / "config.yaml").read_text()
    assert mcp.ensure_teammate_mcp(root) == 2
    for d in (a, b):
        servers = yaml.safe_load((d / "config.yaml").read_text())["mcp_servers"]
        assert servers[bot_relay.TEAMMATES_MCP_SERVER]["args"] == ["-m", "tools.message_agent_mcp"]
        assert yaml.safe_load((d / "config.yaml").read_text())["model"] == {"default": "x"}
    assert "mcp_servers" not in (plain / "config.yaml").read_text()
    assert (root / "config.yaml").read_text() == default_before
    assert mcp.ensure_teammate_mcp(root) == 0  # idempotent


def test_packaged_entry_uses_engine_expanded_variables(monkeypatch):
    monkeypatch.setenv("FACTR_BACKEND_PYTHON", "/app/py")
    e = mcp.desired_server_entry()
    assert e["command"] == "${FACTR_BACKEND_PYTHON}" and e["env"]["PYTHONPATH"] == "${FACTR_BACKEND_PYTHONPATH}"
    assert e["timeout_secs"] >= 600


def test_python_loop_skips_the_engine_side_server(root, monkeypatch):
    import tools.mcp_tool_config as cfg

    (root / "config.yaml").write_text(yaml.safe_dump({"mcp_servers": {
        bot_relay.TEAMMATES_MCP_SERVER: {"command": "x"}, "other": {"command": "y"}}}), encoding="utf-8")
    assert set(cfg._load_mcp_config()) >= {"other"}
    assert bot_relay.TEAMMATES_MCP_SERVER not in cfg._load_mcp_config()


def test_protocol_handshake_list_and_call(root, monkeypatch):
    _profile(root, "alice")
    _profile(root, "bob")
    init = mcp.handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}, root)
    assert init["result"]["protocolVersion"] == "2025-03-26" and init["result"]["capabilities"] == {"tools": {}}
    assert mcp.handle({"jsonrpc": "2.0", "method": "notifications/initialized"}, root) is None
    tools = mcp.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, root)["result"]["tools"]
    assert [t["name"] for t in tools] == ["message_agent"]
    assert "alice" in tools[0]["description"] and "bob" in tools[0]["description"]
    monkeypatch.setattr(mcp, "deliver", lambda t, m, h=None: (f"reply to {t}: {m}", True))
    out = mcp.handle({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                      "params": {"name": "message_agent", "arguments": {"target": "bob", "message": "hi"}}}, root)
    assert out["result"] == {"content": [{"type": "text", "text": "reply to bob: hi"}], "isError": False}
    bad = mcp.handle({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "nope"}}, root)
    assert bad["error"]["code"] == -32602


def test_deliver_reaches_the_teammate_and_returns_the_reply(root, tmp_path, monkeypatch):
    """A -> B through the real runner (lock, dm file, author env) with a stub standing in for the chat CLI."""
    alice, bob = _profile(root, "alice"), _profile(root, "bob")
    stub = tmp_path / "stubcli.py"
    stub.write_text(textwrap.dedent("""
        import os, sys
        a = sys.argv[1:]
        q = a[a.index("--query-file") + 1]
        print("B-REPLY profile=%s author=%s got=%s" % (a[a.index("-p") + 1], os.environ.get("FACTR_TURN_AUTHOR", "")[:40], open(q).read()))
    """), encoding="utf-8")
    monkeypatch.setattr(bot_relay, "bot_cli_prefix", lambda: [sys.executable, str(stub)])
    monkeypatch.setenv("PYTHONPATH", ":".join([str(Path(mcp.__file__).resolve().parent.parent), *[p for p in sys.path if p]]))
    text, ok = mcp.deliver("bob", "please summarise X", alice)
    assert ok, text
    assert "B-REPLY profile=bob" in text and "please summarise X" in text and "(@alice)" in text
    text, ok = mcp.deliver("nobody", "hi", alice)
    assert not ok and "No teammate named 'nobody'" in text
    assert mcp.deliver("alice", "hi", alice)[1] is False


def test_stdio_server_speaks_newline_json_rpc(root):
    _profile(root, "alice")
    env = {"FACTR_CONFIG_HOME": str(root / "profiles" / "alice"), "PATH": "/usr/bin:/bin",
           "PYTHONPATH": str(Path(mcp.__file__).resolve().parent.parent) + ":" + ":".join(p for p in sys.path if p)}
    reqs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}]
    proc = subprocess.run([sys.executable, "-m", "tools.message_agent_mcp"], input="\n".join(map(json.dumps, reqs)) + "\n",
                          capture_output=True, text=True, env=env, timeout=60, cwd=str(root))
    lines = [json.loads(x) for x in proc.stdout.splitlines()]
    assert [r["id"] for r in lines] == [1, 2], proc.stderr
