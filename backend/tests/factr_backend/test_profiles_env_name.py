"""The engine launcher sets FACTR_PROFILE; the backend reads only that name, with the FACTR_CONFIG_HOME
derivation as the fallback."""

from factr_backend.profiles import current_profile_name


def test_reads_factr_profile(tmp_path, monkeypatch):
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    monkeypatch.setenv("FACTR_PROFILE", "reviewer")
    assert current_profile_name("x") == "reviewer"


def test_old_names_are_ignored(tmp_path, monkeypatch):
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(tmp_path))
    monkeypatch.delenv("FACTR_PROFILE", raising=False)
    monkeypatch.setenv("FACTR_BACKEND_PROFILE", "ghost")
    monkeypatch.setenv("FACTR_PROFILE_NAME", "ghost")
    assert current_profile_name("x") != "ghost"


def test_falls_back_to_config_home_derivation(tmp_path, monkeypatch):
    home = tmp_path / "profiles" / "alpha"
    home.mkdir(parents=True)
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home))
    monkeypatch.delenv("FACTR_PROFILE", raising=False)
    assert current_profile_name("x") == "alpha"


def test_groups_methods_profile_name_follows_derivation(tmp_path, monkeypatch):
    home = tmp_path / "profiles" / "alpha"
    home.mkdir(parents=True)
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home))
    monkeypatch.delenv("FACTR_PROFILE", raising=False)
    from tui_gateway import methods_groups

    assert methods_groups._profile_name() == "alpha"
