"""Firecrawl web search + extract provider (direct SDK or keyless cloud).

Config: ``web.backend`` / ``web.search_backend`` / ``web.extract_backend: firecrawl``.
Env: FIRECRAWL_API_KEY, FIRECRAWL_API_URL (self-hosted).
"""

from __future__ import annotations

import asyncio
import logging
from typing import Any, Dict, List, Optional

import httpx

from plugins.web._common import BaseWebSearchProvider, keyless_extract, keyless_search, lazy_ensure, search_fail, search_ok, setup_schema
from tools.url_safety import is_safe_url
# Module-level (cheap import) so tests can monkeypatch the policy gate on this module.
from tools.website_policy import check_website_access

logger = logging.getLogger(__name__)

_FIRECRAWL_CLOUD_API_URL = "https://api.firecrawl.dev"

# The SDK costs ~200ms of imports on a cold CLI; defer to first use (tests patch ``Firecrawl`` here).
_FIRECRAWL_CLS_CACHE: Optional[type] = None


def _load_firecrawl_cls() -> type:
    """Import and cache ``firecrawl.Firecrawl`` (lazy_deps install hint → ImportError)."""
    global _FIRECRAWL_CLS_CACHE
    if _FIRECRAWL_CLS_CACHE is None:
        lazy_ensure("search.firecrawl")
        from firecrawl import Firecrawl as _cls
        _FIRECRAWL_CLS_CACHE = _cls
    return _FIRECRAWL_CLS_CACHE


class _FirecrawlProxy:
    """Callable proxy that looks like ``firecrawl.Firecrawl`` but imports lazily."""

    __slots__ = ()

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        return _load_firecrawl_cls()(*args, **kwargs)

    def __instancecheck__(self, obj: Any) -> bool:
        return isinstance(obj, _load_firecrawl_cls())

    def __repr__(self) -> str:
        return "<lazy firecrawl.Firecrawl proxy>"


Firecrawl = _FirecrawlProxy()

# --- Client construction (direct) ---------------------------
def _wt():
    """Client cache slots live on tools.web_tools so tests that reset ``_firecrawl_client`` there see it."""
    import tools.web_tools as _mod
    return _mod


def _env(name: str) -> str:
    from factr_backend.config import get_env_value
    return (get_env_value(name) or "").strip()


def _get_direct_firecrawl_config() -> Optional[tuple]:
    """Direct Firecrawl ``(mode, kwargs, cache_key)`` or None. ``mode`` is ``"sdk"`` (keyed / self-hosted) or
    ``"keyless"`` (explicit selection + no credentials → anonymous public cloud; the explicit selection is
    required so an unconfigured install never silently routes to it)."""
    api_key = _env("FIRECRAWL_API_KEY")
    api_url = _env("FIRECRAWL_API_URL").rstrip("/")
    if api_key or api_url:
        return "sdk", {k: v for k, v in (("api_key", api_key), ("api_url", api_url)) if v}, ("direct", api_url or None, api_key or None)
    if _is_explicit_firecrawl_selection():
        return "keyless", {"api_url": _FIRECRAWL_CLOUD_API_URL}, ("direct-keyless", _FIRECRAWL_CLOUD_API_URL, None)
    return None


def _is_explicit_firecrawl_selection() -> bool:
    from plugins.web.keyless_mcp import _web_config_selects
    return _web_config_selects("firecrawl")


def _use_keyless_ring() -> bool:
    """Route via the keyless ring only with no direct credentials and the keyless tier isn't
    disabled or pinned paid."""
    if _env("FIRECRAWL_API_KEY") or _env("FIRECRAWL_API_URL"):
        return False
    from plugins.web.keyless_mcp import use_keyless
    return use_keyless("firecrawl", "")


class _KeylessFirecrawlClient:
    """Minimal REST client for Firecrawl's keyless cloud mode; duck-types the SDK's
    ``search`` / ``scrape`` and never sends an Authorization header."""

    def __init__(self, api_url: str = _FIRECRAWL_CLOUD_API_URL):
        self.api_url = api_url.rstrip("/")

    def _post(self, path: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        response = httpx.post(f"{self.api_url}{path}", json=payload, headers={"Content-Type": "application/json"}, timeout=60.0)
        response.raise_for_status()
        return response.json()

    search = lambda self, *, query, limit=5: self._post("/v2/search", {"query": query, "limit": limit})  # noqa: E731
    scrape = lambda self, *, url, formats: self._post("/v2/scrape", {"url": url, "formats": formats})  # noqa: E731


def check_firecrawl_api_key() -> bool:
    """True when the route selected via ``factr tools`` is usable."""
    return _get_direct_firecrawl_config() is not None


def _get_firecrawl_client() -> Any:
    """Get or create the cached Firecrawl client. Direct Firecrawl only: raises ValueError when
    neither FIRECRAWL_API_KEY nor FIRECRAWL_API_URL is set (and keyless cloud is not selected)."""
    wt = _wt()
    from tools.tool_backend_helpers import read_selection, selection_error
    selected = read_selection("web")
    resolved = _get_direct_firecrawl_config()
    if resolved is None:
        logger.error("Firecrawl client initialization failed: direct Firecrawl config is missing.")
        if selected is not None or _is_explicit_firecrawl_selection():
            raise ValueError(selection_error(
                "web", selected or "firecrawl", "neither FIRECRAWL_API_KEY nor FIRECRAWL_API_URL is set"))
        raise ValueError(
            "Web tools are not configured. Set FIRECRAWL_API_KEY for cloud Firecrawl or set FIRECRAWL_API_URL "
            "for a self-hosted Firecrawl instance.")
    client_mode, kwargs, client_config = resolved
    cached = getattr(wt, "_firecrawl_client", None)
    if cached is not None and getattr(wt, "_firecrawl_client_config", None) == client_config:
        return cached
    wt._firecrawl_client = _KeylessFirecrawlClient(api_url=kwargs["api_url"]) if client_mode == "keyless" else Firecrawl(**kwargs)
    wt._firecrawl_client_config = client_config
    return wt._firecrawl_client


# --- Response shape normalization (SDK / direct differ) --------------
def _to_plain_object(value: Any) -> Any:
    """SDK objects (pydantic ``model_dump`` / ``__dict__``) → plain data when possible."""
    if value is None or isinstance(value, (dict, list, str, int, float, bool)):
        return value
    for attr, convert in (("model_dump", lambda v: v.model_dump()), ("__dict__", lambda v: {k: x for k, x in v.__dict__.items() if not k.startswith("_")})):
        if hasattr(value, attr):
            try:
                return convert(value)
            except Exception:  # noqa: BLE001
                pass
    return value


def _normalize_result_list(values: Any) -> List[Dict[str, Any]]:
    return [p for p in map(_to_plain_object, values) if isinstance(p, dict)] if isinstance(values, list) else []


def _extract_web_search_results(response: Any) -> List[Dict[str, Any]]:
    """Search results across SDK/direct response shapes."""
    plain = _to_plain_object(response)
    if isinstance(plain, dict):
        data = plain.get("data")
        if isinstance(data, list):
            return _normalize_result_list(data)
        candidates = [data.get("web"), data.get("results")] if isinstance(data, dict) else []
        for candidate in candidates + [plain.get("web"), plain.get("results")]:
            normalized = _normalize_result_list(candidate)
            if normalized:
                return normalized
    if hasattr(response, "web"):
        return _normalize_result_list(getattr(response, "web", []))
    return []


def _extract_scrape_payload(scrape_result: Any) -> Dict[str, Any]:
    plain = _to_plain_object(scrape_result)
    if not isinstance(plain, dict):
        return {}
    return plain["data"] if isinstance(plain.get("data"), dict) else plain


def _error_entry(url: str, error: str, *, title: str = "", raw: bool = False, blocked: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    """Per-URL extract failure. ``raw`` adds ``raw_content`` (post-scrape failures carry
    it, pre-scrape ones don't); ``blocked`` adds ``blocked_by_policy``."""
    policy = {"blocked_by_policy": {k: blocked[k] for k in ("host", "rule", "source")}} if blocked else {}
    return {"url": url, "title": title, "content": "", **({"raw_content": ""} if raw else {}), "error": error, **policy}


_SCRAPE_TIMEOUT_MSG = "Scrape timed out after 60s — page may be too large or unresponsive. Try browser_navigate instead."
_UNSAFE_REDIRECT_MSG = "Blocked: URL targets a private or internal network address"


async def _scrape_one(url: str, formats: List[str], format: Optional[str]) -> Dict[str, Any]:
    """Scrape one URL (60s timeout) and re-check SSRF + website policy against the
    post-redirect URL. Never raises for scrape errors; returns an error entry instead."""
    if blocked := check_website_access(url):
        logger.info("Blocked web_extract for %s by rule %s", blocked["host"], blocked["rule"])
        return _error_entry(url, blocked["message"], blocked=blocked)
    try:
        logger.info("Firecrawl scraping: %s", url)
        try:
            scrape_result = await asyncio.wait_for(asyncio.to_thread(_get_firecrawl_client().scrape, url=url, formats=formats), timeout=60)
        except asyncio.TimeoutError:
            logger.warning("Firecrawl scrape timed out for %s", url)
            return _error_entry(url, _SCRAPE_TIMEOUT_MSG)
        payload = _extract_scrape_payload(scrape_result)
        metadata = payload.get("metadata", {})
        # SDK may return a typed object for metadata (raw __dict__ here, unlike _to_plain_object).
        if not isinstance(metadata, dict):
            metadata = metadata.model_dump() if hasattr(metadata, "model_dump") else getattr(metadata, "__dict__", {})
        title, final_url = metadata.get("title", ""), metadata.get("sourceURL", url)
        if not is_safe_url(final_url):
            logger.info("Blocked redirected web_extract for unsafe final URL: %s", final_url)
            return _error_entry(final_url, _UNSAFE_REDIRECT_MSG, title=title, raw=True)
        if final_blocked := check_website_access(final_url):
            logger.info("Blocked redirected web_extract for %s by rule %s", final_blocked["host"], final_blocked["rule"])
            return _error_entry(final_url, final_blocked["message"], title=title, raw=True, blocked=final_blocked)
        markdown, html = payload.get("markdown"), payload.get("html")
        content = markdown if format == "markdown" or (format is None and markdown) else html or markdown or ""
        return {"url": final_url, "title": title, "content": content, "raw_content": content, "metadata": metadata}
    except Exception as scrape_err:  # noqa: BLE001
        logger.debug("Firecrawl scrape failed for %s: %s", url, scrape_err)
        return _error_entry(url, str(scrape_err), raw=True)


class FirecrawlWebSearchProvider(BaseWebSearchProvider):
    """Firecrawl search + extract provider with dual auth paths."""

    NAME = "firecrawl"
    DISPLAY_NAME = "Firecrawl"
    EXTRACT = True
    KEYLESS = True  # default-on ring member unless pinned ``paid``

    def is_available(self) -> bool:
        return check_firecrawl_api_key()

    def search(self, query: str, limit: int = 5) -> Dict[str, Any]:
        """Pre-flight errors (ValueError / ImportError) propagate so the dispatcher emits
        the legacy ``tool_error`` envelope; in-flight errors become failure dicts."""
        from tools.interrupt import is_interrupted
        if is_interrupted():
            return search_fail("Interrupted")
        if _use_keyless_ring():
            return keyless_search("Firecrawl", "firecrawl", query, limit, logger)
        logger.info("Firecrawl search: '%s' (limit=%d)", query, limit)
        client = _get_firecrawl_client()
        try:
            web_results = _extract_web_search_results(client.search(query=query, limit=limit))
            logger.info("Firecrawl: found %d search results", len(web_results))
            return search_ok(web_results)
        except Exception as exc:  # noqa: BLE001
            logger.warning("Firecrawl search error: %s", exc)
            return search_fail(f"Firecrawl search failed: {exc}")

    async def extract(self, urls: List[str], **kwargs: Any) -> List[Dict[str, Any]]:
        """Per-URL scrape; failures become items with an ``error`` field.
        ``format``: "markdown" | "html" | both (markdown preferred)."""
        from tools.interrupt import is_interrupted as _is_interrupted
        if _is_interrupted():
            return [{"url": u, "error": "Interrupted", "title": ""} for u in urls]
        if _use_keyless_ring():
            return await asyncio.to_thread(keyless_extract, "Firecrawl", "firecrawl", urls, logger)
        format = kwargs.get("format")
        formats = [format] if format in ("markdown", "html") else ["markdown", "html"]
        return [
            {"url": url, "error": "Interrupted", "title": ""} if _is_interrupted() else await _scrape_one(url, formats, format)
            for url in urls
        ]

    def get_setup_schema(self) -> Dict[str, Any]:
        return setup_schema(
            "Firecrawl", "keyless/paid",
            "Full search + extract; supports keyless cloud and direct API.",
            "FIRECRAWL_API_KEY", "Firecrawl API key (optional; blank = keyless cloud or self-hosted)", "https://docs.firecrawl.dev/introduction",
        )


# ---- BEGIN PLUGIN-COMPAT (revert-scheduled; see COMPAT_MANIFEST.md) ----
# Names external plugins imported from this module before the Sep 2026 decomposition.
# Internal code MUST NOT use these (scripts/check_compat_pointers.py fails CI if it does).
# The whole block is removed by reverting the commit that added it.
from typing import NoReturn  # noqa: F401,E402
from typing import TYPE_CHECKING  # noqa: F401,E402
import os  # noqa: F401,E402


_PLUGIN_COMPAT_LAZY = {
    'WebSearchProvider': ('agent.web_search_provider', 'WebSearchProvider'),
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
