"""The Factr engine's timer fires cron through POST /api/cron/tick: one tick per profile store."""

import contextlib

import cron.scheduler as scheduler
import cron.scheduler_provider as sp
import factr_backend.profiles as profiles_mod
import factr_backend.web_server_cron as wsc


def test_tick_runs_once_per_profile_home_inside_its_store_scope(monkeypatch, tmp_path):
    homes = [("default", tmp_path / "root"), ("coder", tmp_path / "profiles" / "coder")]
    for _name, home in homes:
        home.mkdir(parents=True)
    monkeypatch.setattr(profiles_mod, "profiles_to_serve", lambda **_kw: list(homes))
    seen = []
    monkeypatch.setattr(scheduler, "tick", lambda **kw: seen.append(kw) or 2)
    entered = []
    real_scope = sp._profile_cron_scope

    @contextlib.contextmanager
    def scope(home):
        entered.append(home)
        with real_scope(home):
            yield

    monkeypatch.setattr(sp, "_profile_cron_scope", scope)
    assert wsc._tick_cron_profiles() == 4
    assert entered == [home for _n, home in homes]
    assert all(kw == {"verbose": False, "sync": True} for kw in seen)
