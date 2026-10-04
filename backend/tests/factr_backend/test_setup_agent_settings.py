"""Tests for agent-settings copy in the interactive setup wizard."""

from factr_backend.setup import setup_agent_settings




def test_setup_agent_settings_prefers_config_over_stale_env(tmp_path, monkeypatch, capsys):
    """Config.yaml wins even when a stale .env value disagrees.

    Regression guard for the bug where `.env FACTR_MAX_ITERATIONS=60`
    from an old `factr setup` run shadowed `agent.max_turns: 500` in
    config.yaml. The wizard must now display the config value.
    """
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))

    config = {
        "agent": {"max_turns": 500},  # user bumped this in config.yaml
        "display": {"tool_progress": "all"},
        "compression": {"threshold": 0.50},
    }

    prompt_answers = iter(["500", "all", "0.5"])

    # Simulate stale .env value — the wizard must ignore this.
    monkeypatch.setattr(
        "factr_backend.setup.get_env_value",
        lambda key: "60" if key == "FACTR_MAX_ITERATIONS" else "",
    )
    monkeypatch.setattr("factr_backend.setup.prompt", lambda *args, **kwargs: next(prompt_answers))
    monkeypatch.setattr("factr_backend.setup.prompt_choice", lambda *args, **kwargs: 4)
    monkeypatch.setattr("factr_backend.setup.save_env_value", lambda *args, **kwargs: None)

    removed_keys: list[str] = []
    monkeypatch.setattr(
        "factr_backend.setup.remove_env_value",
        lambda key: (removed_keys.append(key), True)[1],
    )
    monkeypatch.setattr("factr_backend.setup.save_config", lambda *args, **kwargs: None)

    setup_agent_settings(config)

    out = capsys.readouterr().out
    # Config value wins
    assert "Press Enter to keep 500." in out
    assert "Press Enter to keep 60." not in out
    # And the stale .env entry gets cleaned up
    assert "FACTR_MAX_ITERATIONS" in removed_keys
