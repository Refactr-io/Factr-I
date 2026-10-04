"""Import-safety tests for the Discord gateway adapter."""

import builtins
import importlib
import sys


class TestDiscordImportSafety:
    def test_module_imports_even_when_discord_dependency_is_missing(self, monkeypatch):
        original_import = builtins.__import__

        def fake_import(name, globals=None, locals=None, fromlist=(), level=0):
            if name == "discord" or name.startswith("discord."):
                raise ImportError("discord unavailable for test")
            return original_import(name, globals, locals, fromlist, level)

        # Purge the cached module so the import below actually re-runs the
        # module body with discord.py simulated-missing.
        # The re-import also rebinds the `adapter`/`discord` attributes on the parent packages; restore it at teardown so
        # later tests that do `from plugins.platforms.discord import adapter` don't see the discord=None copy.
        pkg = importlib.import_module("plugins.platforms.discord")
        monkeypatch.setattr(importlib.import_module("plugins.platforms"), "discord", pkg)
        monkeypatch.setattr(pkg, "adapter", importlib.import_module("plugins.platforms.discord.adapter"))
        monkeypatch.delitem(sys.modules, "plugins.platforms.discord.adapter", raising=False)
        monkeypatch.delitem(sys.modules, "plugins.platforms.discord", raising=False)
        monkeypatch.setattr(builtins, "__import__", fake_import)

        module = importlib.import_module("plugins.platforms.discord.adapter")

        assert module.DISCORD_AVAILABLE is False
        assert module.discord is None
