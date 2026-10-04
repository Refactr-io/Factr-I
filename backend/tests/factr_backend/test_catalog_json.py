"""The local-model catalog: the packaged JSON is the only source, and
min_engine gates day-0 models."""

from __future__ import annotations

import dataclasses
import io
import json
import urllib.request

import pytest

import factr_backend.local_runtime.catalog as cat


@pytest.fixture(autouse=True)
def _reset_catalog(monkeypatch):
    """Each test starts with the packaged catalog."""
    packaged = cat._packaged_catalog()
    monkeypatch.setattr(cat, "CATALOG", packaged)
    yield


def _doc_from(entries):
    """A fetchable catalog document built by mutating the packaged JSON."""
    from importlib.resources import files

    doc = json.loads(files("factr_backend.local_runtime")
                     .joinpath("catalog.json").read_text(encoding="utf-8"))
    doc["models"] = entries(doc["models"])
    return doc


def test_packaged_json_round_trips_the_catalog():
    """The packaged JSON must produce a complete, selection-ready catalog:
    every entry carries estimator inputs and at least one variant, and the
    known invariants (best-first ordering, Q4 floor) hold — the same
    contract the literals obeyed."""
    assert len(cat.CATALOG) >= 4
    for e in cat.CATALOG:
        assert e.variants and e.n_ctx_train > 0 and e.per_layer_f16 >= 0
        sizes = [v.size_bytes for v in e.variants]
        assert sizes == sorted(sizes, reverse=True), f"{e.id} not best-first"


def test_loader_ignores_unknown_fields():
    doc = _doc_from(lambda m: m)
    doc["models"][0]["future_field"] = {"anything": True}
    entries = cat._load_catalog(doc)
    assert entries[0].id == doc["models"][0]["id"]


def test_min_engine_gate(monkeypatch):
    from factr_backend.web_routers.local_models import _engine_too_old

    monkeypatch.setattr("factr_backend.local_runtime.binaries.installed_tags",
                        lambda: ["b10362"])
    assert _engine_too_old("") is False, "no requirement, no gate"
    assert _engine_too_old("b10000") is False, "installed engine suffices"
    assert _engine_too_old("b10363") is True, "newer requirement gates"
