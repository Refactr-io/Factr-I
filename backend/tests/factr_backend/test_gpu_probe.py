"""Issue #12: real discrete VRAM, no fake unified memory, CUDA selection."""
from unittest import mock

from factr_backend.local_runtime import binaries, bootstrap, gpu_probe, hardware

GIB = 1 << 30
REG = ("Intel(R) UHD Graphics 770|1073741824\n"
       "NVIDIA GeForce RTX 4060|8585740288\n")


def test_registry_picks_discrete_not_igpu():
    gpu = gpu_probe.pick_discrete(gpu_probe.parse_registry_gpus(REG))
    assert gpu.name == "NVIDIA GeForce RTX 4060" and gpu.vendor == "nvidia"
    assert gpu.vram_bytes == 8585740288


def test_igpu_only_is_none_and_garbage_ignored():
    assert gpu_probe.pick_discrete(gpu_probe.parse_registry_gpus("Intel(R) UHD Graphics 770|1073741824\nx|abc")) is None


def test_arc_and_radeon_rx_are_discrete():
    assert not gpu_probe.is_integrated("Intel(R) Arc(TM) A770 Graphics")
    assert not gpu_probe.is_integrated("AMD Radeon RX 7800 XT")
    assert gpu_probe.is_integrated("AMD Radeon(TM) Graphics")


def _budget(smi, dgpu, apple=False):
    with mock.patch.object(hardware, "_ram_bytes", return_value=(32 * GIB, 20 * GIB)), \
         mock.patch.object(hardware, "_nvidia_vram", return_value=smi), \
         mock.patch.object(hardware, "_unified_pool_bytes", return_value=None), \
         mock.patch.object(hardware, "_discrete_gpu_fallback", return_value=dgpu), \
         mock.patch.object(gpu_probe, "is_apple_silicon", return_value=apple):
        return hardware.probe_budget(planning=True)


def test_nvml_failure_uses_registry_vram_not_ram():
    b = _budget(None, gpu_probe.GpuInfo("NVIDIA GeForce RTX 4060", 8 * GIB))
    assert not b.uma and b.total_device_bytes == 8 * GIB
    assert b.usable_vram_bytes < 8 * GIB  # margin carved off


def test_no_gpu_non_apple_is_not_unified():
    b = _budget(None, None)
    assert not b.uma and b.usable_vram_bytes == 0 and b.ram_available_bytes == 32 * GIB


def test_apple_silicon_is_unified():
    assert _budget(None, None, apple=True).uma


def test_vendor_falls_back_to_registry_and_selects_cuda():
    with mock.patch.object(hardware, "_nvidia_smi_path", return_value=None), \
         mock.patch.object(hardware, "_discrete_gpu_fallback",
                           return_value=gpu_probe.GpuInfo("NVIDIA GeForce RTX 4060", 8 * GIB)):
        vendor = bootstrap._detect_gpu_vendor()
    assert binaries.select_backend(vendor, "win") == "cuda"
