"""Contracts: config, setup readiness, model inventory, connectors,
image generation and structured session control.

Handlers: ``tui_gateway/methods_config.py`` (``config.get``, ``setup.*``),
``methods_config_set.py`` (``config.set``), ``methods_complete.py``
(``model.options``), ``methods_connectors.py``, ``methods_images.py``, ``methods_session_control.py``
and ``methods_session.py`` (``verification.status``).
"""

from __future__ import annotations

from typing import Literal

from pydantic import Field

from .base import JsonValue, Params, Result, WireEnum
from .common import OpenModel, ProfileParams, SessionLiveInfo
from .registry import method

# ── config.get ────────────────────────────────────────────────────────────────────────────────


class ConfigGetParams(ProfileParams):
    """``key`` selects one getter from ``_CONFIG_GETTERS``; ``cwd`` feeds the ``project`` getter,
    ``session_id`` lets ``reasoning`` / ``fast`` answer with the session's live pin."""

    key: str
    cwd: str | None = None
    session_id: str | None = None


class ConfigProviderRef(OpenModel):
    """``factr_backend/models.py::list_available_providers`` row."""

    id: str
    label: str
    aliases: list[str] = Field(default_factory=list)
    authenticated: bool = False


class ConfigGetResult(Result):
    """Union of every getter's payload: ``value`` for the simple words, ``config`` for ``full``,
    ``mtime`` / ``mcp_rev`` for the poller, ``model`` / ``provider`` / ``providers`` for ``provider``,
    ``home`` / ``display`` for ``profile``, ``cwd`` / ``branch`` for ``project``, ``prompt``."""

    value: str | None = None
    display: str | None = None
    tool_progress: str | None = None
    model: str | None = None
    provider: str | None = None
    providers: list[ConfigProviderRef] | None = None
    home: str | None = None
    cwd: str | None = None
    branch: str | None = None
    config: dict[str, JsonValue] | None = None
    prompt: str | None = None
    mtime: float | None = None
    mcp_rev: str | None = None


method("config.get", params=ConfigGetParams, result=ConfigGetResult,
       doc="Read one normalised config value (or the whole effective config) the way the UIs render it.")


# ── config.set ────────────────────────────────────────────────────────────────────────────────


class ConfigSetScope(WireEnum):
    session = "session"
    global_ = "global"
    once = "once"


class ConfigSetParams(ProfileParams):
    """``key`` picks the setter (``_CONFIG_SETTERS``, ``details_mode.<section>``, display toggles);
    ``value`` is the raw word/string the setter normalises (falsy non-strings are reported back in
    the error). ``scope`` applies to ``yolo`` / ``reasoning``; ``confirm_expensive_model`` to ``model``."""

    key: str
    value: JsonValue = ""
    session_id: str | None = None
    scope: str | None = None
    confirm_expensive_model: bool = False


class ConfigSetResult(Result):
    """``{key, value}`` plus the setter's extras: model switches add ``warning`` /
    ``confirm_required`` / ``confirm_message`` / ``scope`` / ``deferred``; ``focus`` adds
    ``tool_progress``; ``cwd`` adds ``cwd`` / ``branch``; ``personality`` adds ``history_reset`` /
    ``info``; ``yolo`` reports its ``scope``. ``value`` is a bool only for the display toggles."""

    key: str
    value: str | bool | None = None
    warning: str | None = None
    confirm_required: bool | None = None
    confirm_message: str | None = None
    scope: str | None = None
    deferred: bool | None = None
    tool_progress: str | None = None
    cwd: str | None = None
    branch: str | None = None
    history_reset: bool | None = None
    info: SessionLiveInfo | None = None


method("config.set", params=ConfigSetParams, result=ConfigSetResult,
       doc="Change one config key (persisted or session-scoped) and read back the normalised value.")


# ── setup readiness ───────────────────────────────────────────────────────────────────────────


class SetupStatusResult(Result):
    """``provider_configured`` is the loose answer; the boot record's fields (``ready``,
    ``other_providers``, ``inference_provider``) ride along on the launch profile.
    An unknown ``profile`` answers ``ok=False`` + ``error``."""

    provider_configured: bool | None = None
    ready: bool | None = None
    other_providers: bool | None = None
    inference_provider: str | None = None
    profile: str | None = None
    ok: bool | None = None
    error: str | None = None


method("setup.status", params=ProfileParams, result=SetupStatusResult,
       doc="Loose provider check: is ANY provider auth state discoverable for the (launch or named) profile.")


class SetupRuntimeCheckParams(ProfileParams):
    provider: str | None = None


class SetupRuntimeCheckResult(Result):
    """``ok=False`` + ``error`` when the resolved model can't be served."""

    ok: bool
    provider: str | None = None
    model: str | None = None
    source: str | None = None
    error: str | None = None
    profile: str | None = None


method("setup.runtime_check", params=SetupRuntimeCheckParams, result=SetupRuntimeCheckResult,
       doc="Strict provider check through the same runtime resolution the agent uses on session creation.")


# ── model.options ─────────────────────────────────────────────────────────────────────────────


class ModelOptionsParams(ProfileParams):
    session_id: str | None = None
    explicit_only: bool = False
    include_unconfigured: bool = False
    refresh: bool = False


# TODO(common): ModelPricing / ModelCapabilities / ModelOptionProvider are also the row shape of
# ``model.save_key``'s ``provider`` — the parent consolidates into contracts/common.py.
class ModelPricing(Result):
    """``factr_backend/inventory.py::_apply_pricing`` — formatted $/Mtok strings (``""`` unknown,
    ``"free"``)."""

    input: str
    output: str
    cache: str | None = None
    free: bool


class ModelCapabilities(Result):
    """``factr_backend/inventory.py::_apply_capabilities``."""

    fast: bool
    reasoning: bool
    can_disable_reasoning: bool | None = None


class ModelOptionProvider(OpenModel):
    """One ``factr_backend/inventory.py::build_models_payload`` provider row (the union of every field
    the builder sets; ``pricing_pending`` marks the cached-only path)."""

    slug: str
    name: str
    models: list[str] = Field(default_factory=list)
    total_models: int | None = None
    is_current: bool | None = None
    is_user_defined: bool | None = None
    source: str | None = None
    aliases: list[str] | None = None
    api_url: str | None = None
    auth_type: str | None = None
    authenticated: bool | None = None
    key_env: str | None = None
    warning: str | None = None
    featured_models: list[str] | None = None
    capabilities: dict[str, ModelCapabilities] | None = None
    pricing: dict[str, ModelPricing] | None = None
    pricing_pending: bool | None = None


class ModelOptionsResult(Result):
    providers: list[ModelOptionProvider]
    model: str = ""
    provider: str = ""
    reasoning_effort: str | None = None


method("model.options", params=ModelOptionsParams, result=ModelOptionsResult,
       doc="Provider/model inventory for the picker, layered over the session's live provider when given.")


# ── image.generate ────────────────────────────────────────────────────────────────────────────


class ImageGenerateParams(Params):
    prompt: str | None = None
    aspect_ratio: str | None = None
    probe: JsonValue | None = None  # truthy word/flag: availability check only
    max_bytes: int | None = None


class ImageGenerateResult(Result):
    """``probe`` answers ``{available}`` alone; ``image_data`` (data URL) is omitted when the
    download failed or exceeded ``max_bytes`` so callers fall back to ``image``."""

    available: bool
    success: bool | None = None
    image: str | None = None
    image_data: str | None = None
    error: str | None = None


method("image.generate", params=ImageGenerateParams, result=ImageGenerateResult,
       doc="Generate an image through the tool's provider dispatcher and hand the renderer a data URL.")


# ── session.control ───────────────────────────────────────────────────────────────────────────


class GoalContractSnapshot(Result):
    """``factr_backend/goals.py::GoalContract.to_dict``."""

    outcome: str = ""
    verification: str = ""
    constraints: str = ""
    boundaries: str = ""
    stop_when: str = ""


class GoalGateSnapshot(Result):
    command: str
    timeout_seconds: int
    max_retries: int
    attempts: int
    last_exit_code: int | None = None


class WaitBarrierUntil(Result):
    type: Literal["until"]
    until_at: float
    reason: str = ""


class WaitBarrierTarget(Result):
    type: Literal["session", "pid"]
    target: str | int
    reason: str = ""


class GoalSnapshot(Result):
    """``methods_session_control.py::_safe_goal_snapshot`` — the frontend-safe GoalState subset."""

    title: str
    status: str
    turns_used: int
    max_turns: int
    contract: GoalContractSnapshot
    subgoals: list[str]
    gates: list[GoalGateSnapshot]
    created_at: float | None = None
    updated_at: float | None = None
    paused_reason: str | None = None
    last_verdict: str | None = None
    last_reason: str | None = None
    wait_barrier: WaitBarrierUntil | WaitBarrierTarget | None = Field(default=None, discriminator="type")


class LoopSnapshot(Result):
    """``_safe_loop_snapshot`` — persisted LoopState fields, never its route."""

    prompt: str
    status: str
    mode: str
    interval_seconds: float
    current_delay: float
    times: int
    until: str
    max_ticks: int
    ticks_fired: int
    created_at: float
    last_fired_at: float
    next_due_at: float
    awaiting_response: bool
    deferred_by_goal: bool
    paused_reason: str | None = None
    last_stop_reason: str | None = None


class HeartbeatSnapshot(Result):
    prompt: str
    status: str
    interval_seconds: int
    created_at: float
    last_fired_at: float
    fire_count: int


class SessionControlSnapshot(Result):
    """``_snapshot_control`` — ``revision`` is a hash of the visible state (``""`` when empty);
    ``updated_at`` is the newest persisted timestamp (``0`` when none)."""

    goal: GoalSnapshot | None
    loop: LoopSnapshot | None
    heartbeat: HeartbeatSnapshot | None
    revision: str
    updated_at: float


class SessionControlReadParams(ProfileParams):
    session_id: str


class SessionControlReadResult(Result):
    control: SessionControlSnapshot


method("session.control.read", params=SessionControlReadParams, result=SessionControlReadResult,
       doc="Stable, allowlisted snapshot of one live session's goal / loop / heartbeat state.")


class SessionControlAction(WireEnum):
    goal_pause = "goal.pause"
    goal_resume = "goal.resume"
    goal_clear = "goal.clear"
    goal_unwait = "goal.unwait"
    loop_pause = "loop.pause"
    loop_resume = "loop.resume"
    loop_stop = "loop.stop"
    subgoal_add = "subgoal.add"
    subgoal_remove = "subgoal.remove"
    subgoal_clear = "subgoal.clear"
    heartbeat_pause = "heartbeat.pause"
    heartbeat_resume = "heartbeat.resume"
    heartbeat_clear = "heartbeat.clear"


class SessionControlArgs(Params):
    """``subgoal.add`` reads ``text``; ``subgoal.remove`` reads the 1-based ``index``."""

    text: str | None = None
    index: int | None = None


class SessionControlParams(ProfileParams):
    """``action`` is validated by the handler (unknown / gate actions answer ``4004``), so it stays a
    string on the wire; ``SessionControlAction`` lists the accepted set."""

    session_id: str
    action: str
    args: SessionControlArgs | None = None


class SessionControlDispatch(Result):
    """``_dispatch_envelope`` — the command result's user-visible envelope, every key always present."""

    type: str | None
    output: str | None
    notice: str | None
    message: str | None
    display: str | None


class SessionControlResult(Result):
    control: SessionControlSnapshot
    dispatch: SessionControlDispatch


method("session.control", params=SessionControlParams, result=SessionControlResult,
       doc="Run one allowlisted goal / loop / subgoal / heartbeat action and return the exact resulting snapshot.")


# ── verification.status ───────────────────────────────────────────────────────────────────────


class VerificationStatusParams(ProfileParams):
    session_id: str | None = None
    session_key: str | None = None
    cwd: str | None = None


class VerificationEvidenceRow(OpenModel):
    """One ``verification_events`` row (``agent/verification_evidence.py``)."""

    id: int | None = None
    created_at: str | None = None
    session_id: str | None = None
    cwd: str | None = None
    root: str | None = None
    command: str | None = None
    canonical_command: str | None = None
    kind: str | None = None
    scope: str | None = None
    status: str | None = None
    exit_code: int | None = None
    output_summary: str | None = None


class VerificationStatusInfo(Result):
    """``verification_status()``: ``disabled`` / ``not_applicable`` / ``unverified`` / ``stale`` or the
    latest event's own status; ``root`` and friends only once a workspace was identified."""

    status: str
    evidence: VerificationEvidenceRow | None = None
    root: str | None = None
    session_id: str | None = None
    changed_paths: list[str] | None = None


class VerificationStatusResult(Result):
    verification: VerificationStatusInfo


method("verification.status", params=VerificationStatusParams, result=VerificationStatusResult,
       doc="Best known verification evidence for a cwd/session; read-only, never runs checks.")
