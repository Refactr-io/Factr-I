"""Isolate the state.db repair tests from the host's free disk space.

The repair path fails closed when the volume is nearly full (headroom is a percentage of the
disk), so on a full developer disk every repair test would refuse to run. Tests that exercise
the low-disk refusal install their own ``shutil.disk_usage`` stub, which overrides this one.
"""

import shutil
from collections import namedtuple

import pytest

_Usage = namedtuple("_Usage", "total used free")


@pytest.fixture(autouse=True)
def _roomy_disk(monkeypatch):
    monkeypatch.setattr(shutil, "disk_usage", lambda _path: _Usage(1 << 40, 1 << 39, 1 << 39))
