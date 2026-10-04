"""Unit tests for messaging-gateway notice rendering.

Covers render_notice_line — the pure helper that turns an AgentNotice into the
single plaintext line pushed standalone over a messaging platform (no status
bar, unlike the TUI). Behavior contracts, not data snapshots.
"""
from dataclasses import dataclass


@dataclass
class AgentNotice:
    text: str
    level: str = "info"
    kind: str = "sticky"
    ttl_ms: object = None
    key: str = ""
    id: str = ""
from gateway.run import render_notice_line


# ── Delivery seam: a rendered notice line goes out via _deliver_platform_notice ──

from unittest.mock import AsyncMock, MagicMock

import pytest


def _make_source(platform_value="telegram", chat_id="555", user_id="u1"):
    src = MagicMock()
    plat = MagicMock()
    plat.value = platform_value
    src.platform = plat
    src.chat_id = chat_id
    src.user_id = user_id
    # Real SessionSource.profile is None (single-profile) or a str; a MagicMock
    # auto-attribute would read as a truthy "stamped profile" and trip the
    # fail-closed path in _delivery_adapter_for (see AGENTS.md pitfall #17).
    src.profile = None
    return src


def _make_runner_with_adapter(source, adapter):
    from gateway.run import GatewayRunner

    runner = object.__new__(GatewayRunner)
    runner.adapters = {source.platform: adapter}
    runner.config = MagicMock()
    runner.config.get_notice_delivery = MagicMock(return_value="public")
    runner._thread_metadata_for_source = MagicMock(return_value={"thread": "t"})
    return runner


class TestDeliverNoticeLine:
    """The seam between render_notice_line and the platform adapter.

    Proves a rendered credit-notice line reaches adapter.send (public) /
    send_private_notice (private) through the shared _deliver_platform_notice
    rail — the path the gateway notice_callback schedules onto the loop.
    """

    @pytest.mark.asyncio
    async def test_public_delivery_sends_rendered_line(self):
        source = _make_source()
        adapter = MagicMock()
        adapter.send = AsyncMock(return_value=MagicMock(success=True))
        runner = _make_runner_with_adapter(source, adapter)

        line = render_notice_line(
            AgentNotice(text="⚠ Credits 90% used · $20.00 cap", level="warn")
        )
        await runner._deliver_platform_notice(source, line)

        adapter.send.assert_awaited_once()
        args, kwargs = adapter.send.call_args
        assert args[0] == "555"
        # Delivered verbatim — the policy's single glyph, not a doubled one.
        assert args[1] == "⚠ Credits 90% used · $20.00 cap"


