"""An enabled-but-unpaired WhatsApp is a normal first-run state: warn and skip, never exit the gateway."""
import logging
from unittest.mock import AsyncMock, MagicMock

import pytest

from gateway.config import Platform
from gateway.run_startup import GatewayStartupMixin


def _unpaired_adapter(tmp_path):
    from plugins.platforms.whatsapp.adapter import WhatsAppAdapter

    adapter = WhatsAppAdapter.__new__(WhatsAppAdapter)
    adapter.platform = Platform.WHATSAPP
    adapter.config = MagicMock()
    bridge = tmp_path / "bridge.js"
    bridge.write_text("// stub", encoding="utf-8")
    adapter._bridge_script = str(bridge)
    adapter._session_path = tmp_path / "session"  # no creds.json
    adapter._fatal_error_code = None
    adapter._fatal_error_message = None
    adapter._fatal_error_retryable = True
    adapter._write_runtime_status_safe = MagicMock()
    return adapter


@pytest.mark.asyncio
async def test_unpaired_connect_is_a_skip_that_reconnect_paths_park(tmp_path, monkeypatch):
    monkeypatch.setattr("plugins.platforms.whatsapp.adapter.check_whatsapp_requirements", lambda: True)
    adapter = _unpaired_adapter(tmp_path)

    assert await adapter.connect() is False
    assert "not paired" in adapter.startup_skip_message
    # Reconnect paths still see a non-retryable error, so a mid-run unpair is parked, not retried forever.
    assert adapter.fatal_error_code == "whatsapp_not_paired" and adapter.fatal_error_retryable is False


@pytest.mark.asyncio
async def test_gateway_keeps_running_when_the_only_platform_is_unpaired(tmp_path, monkeypatch, caplog):
    monkeypatch.setattr("plugins.platforms.whatsapp.adapter.check_whatsapp_requirements", lambda: True)
    adapter = _unpaired_adapter(tmp_path)
    await adapter.connect()

    runner = MagicMock()
    runner._safe_adapter_disconnect = AsyncMock()
    runner._update_platform_runtime_status = MagicMock()
    retryable: list = []
    fatal: list = []

    with caplog.at_level(logging.WARNING):
        connected = await GatewayStartupMixin._start_aggregate_connect_results(
            runner, [(Platform.WHATSAPP, adapter, MagicMock(), "failed", None)], retryable, fatal
        )

    assert connected == 0 and retryable == [] and fatal == []
    assert any("skipped" in r.message and r.levelno == logging.WARNING for r in caplog.records)
    runner._update_platform_runtime_status.assert_called_once()
    assert runner._update_platform_runtime_status.call_args.kwargs["platform_state"] == "inactive"

    # Nothing fatal queued -> the startup gate must not request the exit-78 path.
    runner._startup_fail_fatal_config = MagicMock()
    exit_requested = GatewayStartupMixin._start_handle_no_connections(runner, 0, 1, retryable, fatal)
    assert exit_requested is False
    runner._startup_fail_fatal_config.assert_not_called()
