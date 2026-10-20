"""The llamacpp provider row in the model picker payload.

Contract: staged local GGUFs appear as a selectable provider row in
build_models_payload — the same payload /api/model/options and the desktop
picker consume — whenever models are staged, without any credential."""

from __future__ import annotations

import pytest


@pytest.fixture
def factr_home(tmp_path, monkeypatch):
    home = tmp_path / ".factr"
    home.mkdir()
    monkeypatch.setenv("FACTR_CONFIG_HOME", str(home))
    return home


def _stage(home, *names):
    mdir = home / "models"
    mdir.mkdir(exist_ok=True)
    for name in names:
        (mdir / f"{name}.gguf").write_bytes(b"GGUF" + b"\x00" * 64)


def test_no_staged_models_no_row(factr_home):
    from factr_backend.inventory import _local_runtime_row, load_picker_context

    assert _local_runtime_row(load_picker_context()) is None


def test_staged_models_make_a_selectable_row(factr_home):
    from factr_backend.inventory import _local_runtime_row, load_picker_context

    _stage(factr_home, "Qwen3-4B-Instruct-2507-UD-Q8_K_XL", "Some-Other-Model")
    row = _local_runtime_row(load_picker_context())
    assert row is not None
    assert row["slug"] == "llamacpp"
    assert row["authenticated"] is True
    assert "Qwen3-4B-Instruct-2507-UD-Q8_K_XL" in row["models"]
    assert row["total_models"] == 2


def test_row_marks_current_when_config_points_at_llamacpp(factr_home):
    from factr_backend.inventory import _local_runtime_row, load_picker_context

    _stage(factr_home, "M")
    ctx = load_picker_context().with_overrides(current_provider="llamacpp")
    row = _local_runtime_row(ctx)
    assert row is not None and row["is_current"] is True


def test_full_payload_includes_local_row(factr_home):
    """Through the REAL payload builder — the shape the desktop picker eats."""
    from factr_backend.inventory import build_models_payload, load_picker_context

    _stage(factr_home, "Local-Model-X")
    payload = build_models_payload(
        load_picker_context(),
        probe_custom_providers=False,
        probe_current_custom_provider=False,
    )
    slugs = [p["slug"] for p in payload["providers"]]
    assert "llamacpp" in slugs
    row = payload["providers"][slugs.index("llamacpp")]
    assert row["models"] == ["Local-Model-X"]


def test_explicit_only_filter_keeps_local_row_on_any_profile(factr_home):
    """The desktop dropdown requests explicit_only=True, and the local row
    has no config credential by design (credential is reachability). The
    filter must treat staged models as explicit configuration — otherwise
    the row only survives on the profile whose config points at llamacpp,
    and every other profile's dropdown silently loses local models."""
    from factr_backend.inventory import _filter_explicit_provider_rows, _local_runtime_row, load_picker_context

    _stage(factr_home, "Qwen3.8-27B-UD-Q5_K_XL")
    ctx = load_picker_context()
    row = _local_runtime_row(ctx)
    assert row is not None

    # Simulate a profile whose current provider is a cloud one (the normal
    # profile's shape): explicit-only filtering must keep the local row.
    import dataclasses

    ctx = dataclasses.replace(ctx, current_provider="anthropic")
    kept = _filter_explicit_provider_rows([row], ctx)
    assert kept, "explicit-only filter dropped the local-runtime row"
    assert kept[0]["slug"] == "llamacpp"


def test_configured_default_listed_even_when_not_staged(factr_home):
    """Issue #11: the model config points at must show in the picker (status 'unloaded' or not staged)."""
    from factr_backend.inventory import _local_runtime_row, load_picker_context

    ctx = load_picker_context().with_overrides(current_provider="llamacpp", current_model="Qwen3.6-35B-A3B-UD-Q4_K_M")
    row = _local_runtime_row(ctx)
    assert row is not None and row["models"] == ["Qwen3.6-35B-A3B-UD-Q4_K_M"]


def test_managed_router_models_listed_regardless_of_status(factr_home, monkeypatch):
    from factr_backend import inventory
    from factr_backend.inventory import _local_runtime_row, load_picker_context

    monkeypatch.setattr("factr_backend.local_runtime.endpoint.managed_root", lambda: ("http://x", "k"))
    monkeypatch.setattr("factr_backend.local_runtime.endpoint.managed_get_json", lambda *a: {
        "data": [{"id": "A", "status": {"value": "unloaded"}}, {"id": "B", "status": {"value": "loaded"}}]})
    row = _local_runtime_row(load_picker_context())
    assert row["models"] == ["A", "B"]


def test_running_ollama_is_listed(factr_home, monkeypatch):
    from factr_backend.inventory import build_model_options_payload, load_picker_context

    monkeypatch.setattr("factr_backend.models_local.fetch_ollama_local_models", lambda *a, **k: ["qwen3:8b"])
    rows = build_model_options_payload(load_picker_context(), explicit_only=True)["providers"]
    ollama = next(r for r in rows if r["slug"] == "ollama")
    assert ollama["models"] == ["qwen3:8b"] and ollama["authenticated"] is True


def test_no_ollama_no_row(factr_home, monkeypatch):
    from factr_backend.inventory import build_model_options_payload, load_picker_context

    monkeypatch.setattr("factr_backend.models_local.fetch_ollama_local_models", lambda *a, **k: None)
    rows = build_model_options_payload(load_picker_context(), explicit_only=True)["providers"]
    assert not any(r["slug"] == "ollama" for r in rows)
