from agent import curator
from tools import skills_hub, skills_tool


def test_engine_skill_store_follows_factr_home_alone(tmp_path, monkeypatch):
    # The CLI passthrough and the engine both export FACTR_HOME; nothing else selects the one skills dir.
    monkeypatch.setenv("FACTR_HOME", str(tmp_path / ".factr/engine"))
    monkeypatch.delenv("FACTR_ENGINE_URL", raising=False)
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path / ".factr"))

    expected = tmp_path / ".factr/engine" / "skills"
    assert skills_hub._skills_dir() == expected
    assert skills_tool._skills_dir() == expected
    assert curator._state_file() == expected / ".curator_state"


def test_standalone_factr_keeps_its_own_skills_dir(tmp_path, monkeypatch):
    monkeypatch.delenv("FACTR_HOME", raising=False)
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path / ".factr"))
    assert skills_tool._skills_dir() == tmp_path / ".factr" / "skills"
