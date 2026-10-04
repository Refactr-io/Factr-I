"""Skills dashboard routes.

Skills CRUD routes.  Shared helpers are reached via the late-binding seam so
``monkeypatch.setattr(<owning module>, ...)`` keeps working.
"""

import asyncio
from typing import Optional

from fastapi import APIRouter, HTTPException

from factr_backend.web_deps import late
from factr_backend.web_models import SkillContentUpdate, SkillCreate, SkillToggle
from factr_backend.web_routers._common import _profile_scope, config_write_scope, scoped_to_thread

router = APIRouter()

_config_profile_scope = late("_config_profile_scope", "factr_backend.web_server_profiles")
load_config = late("load_config", "factr_backend.config")


def _clear_skills_prompt_cache() -> None:
    """Best-effort: invalidate the skills system-prompt snapshot after a write.

    Mirrors what ``skill_manage`` does so a dashboard-authored skill is picked
    up by the next session without a manual cache reset.
    """
    try:
        from agent.prompt_builder import clear_skills_system_prompt_cache
        clear_skills_system_prompt_cache(clear_snapshot=True)
    except Exception:
        pass


@router.get("/api/skills")
async def get_skills(profile: Optional[str] = None):
    from tools.skills_tool import _find_all_skills
    from factr_backend.skills_config import get_disabled_skills
    from tools.skill_usage import (
        _read_bundled_manifest_names, activity_count, load_usage)

    def _run():
        with _profile_scope(profile):
            config = load_config()
            disabled = get_disabled_skills(config)
            skills = _find_all_skills(skip_disabled=True)
            usage = load_usage()
            # Set-based provenance (same classification as skill_usage.provenance,
            # without a per-skill manifest read): bundled > agent, where
            # "agent" covers agent-authored AND local hand-made skills — the ones
            # the user may edit/delete from the UI.
            bundled_names = _read_bundled_manifest_names()
        for s in skills:
            s["enabled"] = s["name"] not in disabled
            s["usage"] = activity_count(usage.get(s["name"], {}))
            s["provenance"] = "bundled" if s["name"] in bundled_names else "agent"
        return skills

    return await asyncio.to_thread(_run)


@router.put("/api/skills/toggle")
async def toggle_skill(body: SkillToggle, profile: Optional[str] = None):
    from factr_backend.skills_config import get_disabled_skills, save_disabled_skills

    def _run():
        with config_write_scope(body.profile or profile):
            config = load_config()
            disabled = get_disabled_skills(config)
            if body.enabled:
                disabled.discard(body.name)
            else:
                disabled.add(body.name)
            save_disabled_skills(config, disabled)
        return {"ok": True, "name": body.name, "enabled": body.enabled}

    return await asyncio.to_thread(_run)


@router.get("/api/skills/content")
async def get_skill_content(name: str, profile: Optional[str] = None):
    """Raw SKILL.md text for the dashboard editor."""
    from tools.skill_manager_tool import _find_skill

    def _read():
        found = _find_skill(name)
        if not found:
            raise HTTPException(status_code=404, detail=f"Skill '{name}' not found.")
        skill_md = found["path"] / "SKILL.md"
        if not skill_md.exists():
            raise HTTPException(status_code=404, detail=f"Skill '{name}' has no SKILL.md.")
        try:
            content = skill_md.read_text(encoding="utf-8")
        except OSError as exc:
            raise HTTPException(status_code=500, detail=str(exc)) from exc
        return {"name": name, "content": content, "path": str(skill_md)}

    return await scoped_to_thread(profile, _read)


@router.post("/api/skills")
async def create_skill(body: SkillCreate):
    """Create a skill via the agent's ``skill_manage`` write path, minus the
    write-approval gate — an authenticated dashboard write IS the user."""
    from tools.skill_manager_tool import _create_skill

    result = await scoped_to_thread(
        body.profile, lambda: _create_skill(body.name, body.content, body.category or None))
    if not result.get("success"):
        raise HTTPException(status_code=400, detail=result.get("error", "Failed to create skill."))
    _clear_skills_prompt_cache()
    return result


@router.put("/api/skills/content")
async def update_skill_content(body: SkillContentUpdate):
    """Replace the SKILL.md of an existing skill (full rewrite) from the editor."""
    from tools.skill_manager_tool import _edit_skill

    result = await scoped_to_thread(body.profile, lambda: _edit_skill(body.name, body.content))
    if not result.get("success"):
        err = result.get("error", "Failed to update skill.")
        status = 404 if "not found" in str(err).lower() else 400
        raise HTTPException(status_code=status, detail=err)
    _clear_skills_prompt_cache()
    return result


# ---- BEGIN PLUGIN-COMPAT (revert-scheduled; see COMPAT_MANIFEST.md) ----
# Names external plugins imported from this module before the Sep 2026 decomposition.
# Internal code MUST NOT use these (scripts/check_compat_pointers.py fails CI if it does).
# The whole block is removed by reverting the commit that added it.
import logging  # noqa: F401,E402


_PLUGIN_COMPAT_LAZY = {
    'LateState': ('factr_backend.web_deps', 'LateState'),
}


def __getattr__(name):  # PEP 562 — lazy so no import cycles
    target = _PLUGIN_COMPAT_LAZY.get(name)
    if target is None:
        raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
    import importlib
    from factr_backend.plugin_compat import warn_once
    warn_once(__name__, name, *target)
    return getattr(importlib.import_module(target[0]), target[1])
# ---- END PLUGIN-COMPAT ----
