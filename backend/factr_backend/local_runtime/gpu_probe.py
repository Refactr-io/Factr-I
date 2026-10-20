"""Discrete-GPU facts when nvidia-smi is missing or its NVML init fails (issue #12).

Pure parsers plus one Windows probe. Real VRAM only: never Win32_VideoController.AdapterRAM
(32-bit, caps at 4 GB) and never an iGPU's shared system memory.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class GpuInfo:
    name: str
    vram_bytes: int

    @property
    def vendor(self) -> str:
        return gpu_vendor(self.name)


# Integrated parts share system RAM; their registry/DXGI numbers are not VRAM.
_IGPU_RE = re.compile(
    r"\b(uhd|iris|hd graphics|radeon\(tm\) graphics|radeon graphics|vega \d+ graphics|"
    r"basic (render|display)|remote display|virtual)\b", re.I)

_PS_REGISTRY = (
    r"Get-ChildItem 'HKLM:\SYSTEM\CurrentControlSet\Control\Class"
    r"\{4d36e968-e325-11ce-bfc1-08002be10318}' -ErrorAction SilentlyContinue | ForEach-Object { "
    r"$p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue; "
    r"$m = $p.'HardwareInformation.qwMemorySize'; "
    r"if ($p.DriverDesc -and $m) { \"$($p.DriverDesc)|$m\" } }"
)


def gpu_vendor(name: str) -> str:
    low = name.lower()
    if "nvidia" in low or "geforce" in low or "rtx" in low or "quadro" in low:
        return "nvidia"
    if "amd" in low or "radeon" in low:
        return "amd"
    if "intel" in low or "arc" in low.split():
        return "intel"
    return ""


def is_integrated(name: str) -> bool:
    """Intel Arc and Radeon RX are discrete; UHD/Iris/'Radeon Graphics' are iGPUs."""
    return bool(_IGPU_RE.search(name))


def parse_registry_gpus(text: str) -> list[GpuInfo]:
    """Parse 'DriverDesc|qwMemorySize' lines emitted by _PS_REGISTRY."""
    out: list[GpuInfo] = []
    for line in text.splitlines():
        name, sep, raw = line.strip().rpartition("|")
        if sep and name and raw.isdigit() and int(raw) > 0:
            out.append(GpuInfo(name.strip(), int(raw)))
    return out


def pick_discrete(gpus: list[GpuInfo]) -> GpuInfo | None:
    """Largest-VRAM non-integrated adapter, or None when only iGPUs are present."""
    discrete = [g for g in gpus if not is_integrated(g.name)]
    return max(discrete, key=lambda g: g.vram_bytes, default=None)


def windows_discrete_gpu() -> GpuInfo | None:
    """Registry qwMemorySize via PowerShell (64-bit; what DXGI DedicatedVideoMemory reports)."""
    if os.name != "nt":
        return None
    try:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", _PS_REGISTRY],
            capture_output=True, text=True, timeout=15, encoding="utf-8", errors="replace",
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    except (OSError, subprocess.TimeoutExpired):
        return None
    return pick_discrete(parse_registry_gpus(out.stdout))


def is_apple_silicon() -> bool:
    import platform

    return sys.platform == "darwin" and platform.machine() == "arm64"
