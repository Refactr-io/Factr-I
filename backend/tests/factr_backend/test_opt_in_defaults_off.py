"""The opt-in controls added in 0.0.11 ship switched off (or unset), so default behaviour is unchanged."""
from factr_backend.config_defaults import DEFAULT_CONFIG
from factr_backend.web_server_config import CONFIG_SCHEMA


def test_new_controls_are_off_by_default():
    assert DEFAULT_CONFIG["terminal"]["reduce_output"] is False
    assert DEFAULT_CONFIG["tools"]["routing"] is False
    assert DEFAULT_CONFIG["compression"]["deterministic"] is False
    assert DEFAULT_CONFIG["compression"]["llm_second_pass"] is True  # only read when the digest is on
    assert DEFAULT_CONFIG["approvals"]["gui_automation_ask"] is False
    assert DEFAULT_CONFIG["approvals"]["always_ask"] == []
    assert DEFAULT_CONFIG["budget"] == {"max_cost_usd": 0, "max_tokens": 0}
    assert DEFAULT_CONFIG["local_models"] == {"silence_timeout_secs": 0}
    assert DEFAULT_CONFIG["persistence"]["enabled"] is False


def test_every_new_setting_appears_in_the_settings_schema():
    for key in (
        "terminal.reduce_output", "terminal.reduce_output_timeout_ms", "tools.routing", "compression.deterministic",
        "compression.llm_second_pass", "approvals.gui_automation_ask", "approvals.always_ask",
        "budget.max_cost_usd", "budget.max_tokens", "local_models.silence_timeout_secs",
        "persistence.enabled", "persistence.max_nudges", "persistence.budget_fraction",
    ):
        assert CONFIG_SCHEMA[key]["description"] and "→" not in CONFIG_SCHEMA[key]["description"], key
