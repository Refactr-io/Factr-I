"""`factr skills seed`: the one-shot the desktop engine runs when its skill store is empty."""

import json
import sys


def _bundled(tmp_path):
    src = tmp_path / "bundled"
    skill = src / "demo"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text("---\nname: demo\ndescription: A demo skill.\n---\nBody\n")
    return src


def test_seed_copies_bundled_skills_into_the_engine_store_once(tmp_path, monkeypatch, capsys):
    engine_home = tmp_path / "engine"
    monkeypatch.setenv("FACTR_HOME", str(engine_home))
    monkeypatch.setenv("FACTR_BUNDLED_SKILLS", str(_bundled(tmp_path)))
    from factr_backend.skills_hub import do_seed

    do_seed()
    first = json.loads(capsys.readouterr().out.strip().splitlines()[-1])
    assert first == {"copied": 1, "total_bundled": 1}
    assert (engine_home / "skills" / "demo" / "SKILL.md").is_file()

    do_seed()
    second = json.loads(capsys.readouterr().out.strip().splitlines()[-1])
    assert second["copied"] == 0, "an already-seeded store is not copied again"


def test_seed_does_not_re_add_a_skill_the_user_deleted(tmp_path, monkeypatch, capsys):
    engine_home = tmp_path / "engine"
    monkeypatch.setenv("FACTR_HOME", str(engine_home))
    monkeypatch.setenv("FACTR_BUNDLED_SKILLS", str(_bundled(tmp_path)))
    from factr_backend.skills_hub import do_seed
    import shutil

    do_seed()
    shutil.rmtree(engine_home / "skills" / "demo")
    capsys.readouterr()
    do_seed()
    assert json.loads(capsys.readouterr().out.strip().splitlines()[-1])["copied"] == 0
    assert not (engine_home / "skills" / "demo").exists()


def test_cli_routes_skills_seed(monkeypatch):
    from factr_backend.main import main

    seen = {}
    monkeypatch.setattr("factr_backend.skills_hub.do_seed", lambda: seen.setdefault("called", True))
    monkeypatch.setattr(sys, "argv", ["factr", "skills", "seed"])
    main()
    assert seen == {"called": True}
