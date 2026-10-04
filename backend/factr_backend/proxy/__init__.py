"""Local OpenAI-compatible proxy that forwards to OAuth-authenticated upstreams."""

from factr_backend.proxy.adapters.base import UpstreamAdapter

__all__ = ["UpstreamAdapter"]
