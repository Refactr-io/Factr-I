"""The slack plugin must load when the messaging extra (aiohttp) is not installed."""

import importlib
import sys


def test_slack_adapter_imports_without_aiohttp(monkeypatch):
    # A None entry in sys.modules makes `import aiohttp` raise ImportError.
    monkeypatch.setitem(sys.modules, "aiohttp", None)
    name = "plugins.platforms.slack.adapter"
    monkeypatch.delitem(sys.modules, name, raising=False)
    module = importlib.import_module(name)
    try:
        assert module.SLACK_AVAILABLE is False
        assert module.aiohttp is None
    finally:
        sys.modules.pop(name, None)
