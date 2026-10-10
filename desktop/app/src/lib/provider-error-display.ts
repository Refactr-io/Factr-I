// Display-only reading of raw provider/engine error text, for turns whose
// failure arrives without a structured `error_surface` (the engine's own
// provider runtimes send plain strings). Nothing here changes what the engine
// says or does: the raw text is kept intact for "Details" and "Copy error
// details"; this module only picks the plain headline the card and toast show,
// and tidies the Details text (repeats and irrelevant hints).

import type { ErrorSurface } from './error-surface'

// "Claude credentials not available. Run `factr login claude` first." and the
// per-provider variants: no provider has credentials, so nothing is connected.
const NO_CREDENTIALS_RE = /\bcredentials not available\b/i

// The engine's readiness text for a provider with no credential: "No usable credentials found for Claude."
const NO_USABLE_CREDENTIALS_RE = /\bNo usable credentials found for ([^\n]+?)\.(?:\s|$)/i

/** The plain sentence for the engine's "No usable credentials found for <X>." readiness text, or null for
 *  any other text. `savedProvider`: config.yaml's saved provider (`setup.status` `saved_provider`); null
 *  means the user never chose one, so <X> is only the engine's fallback and is not named. Undefined (an
 *  engine that does not report it) keeps the raw text. */
export function noUsableCredentialsMessage(raw: null | string | undefined, savedProvider: null | string | undefined): null | string {
  const provider = raw?.match(NO_USABLE_CREDENTIALS_RE)?.[1]?.trim()

  if (!provider || savedProvider === undefined) {
    return null
  }

  return savedProvider?.trim()
    ? `Your saved provider ${provider} has no usable credentials. Open Settings > Providers to sign in again or choose another.`
    : 'No AI provider is connected yet. Open Settings > Providers (or sign in) to choose one.'
}

const STATUS_PATTERNS: readonly RegExp[] = [
  /\bstatus(?:\s+code)?\s*[:=]?\s*(\d{3})\b/i,
  /\bHTTP(?:\/\d(?:\.\d)?)?\s*(\d{3})\b/i,
  /\((\d{3})\s+[A-Z][A-Za-z ]+\)/,
  /\breturned\s+(\d{3})\b/i,
  /\b(\d{3})\s+(?:Unauthorized|Not Found|Too Many Requests|Internal Server Error|Bad Gateway|Service Unavailable|Gateway Timeout)\b/i
]

// A status code only means "the provider said X" when the text is about a provider call:
// a gateway's own "404 session not found" must not read as a missing model.
const PROVIDER_CONTEXT_RE =
  /\b(?:provider|model|endpoint|chat request|completions?|anthropic|openai|openrouter|ollama|gemini|rate limit)\b/i

const LOCAL_HOST_RE = /\b(?:localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0)\b|\[::1\]|:11434\b/i

const RETRIED_MARK = ' (retried after reloading auth from disk: '

const ENDPOINT_HINT_LINE_RE = /^(?:Local endpoint hint|Ollama hint|LM Studio hint|Hint): .*$/gm

/** The HTTP status a provider error names, or null when it names none. */
export function providerErrorStatus(raw: null | string | undefined): null | number {
  const text = raw ?? ''

  for (const pattern of STATUS_PATTERNS) {
    const status = Number(text.match(pattern)?.[1])

    if (status >= 400 && status <= 599) {
      return status
    }
  }

  return null
}

/** Whether the failing endpoint is on this machine (Ollama, LM Studio, a local server). */
export function isLocalEndpointError(raw: null | string | undefined): boolean {
  const text = raw ?? ''
  const endpoint = text.match(/\b(?:endpoint|base[_ ]url|url)\s*[:=]\s*(\S+)/i)?.[1]

  return LOCAL_HOST_RE.test(endpoint ?? text) || /\bOllama hint\b/.test(text)
}

/** A plain descriptor for a raw provider error, or null when the text is not one we can read. */
export function inferErrorSurface(raw: null | string | undefined): ErrorSurface | null {
  const text = raw?.trim()

  if (!text) {
    return null
  }

  if (NO_CREDENTIALS_RE.test(text)) {
    return { code: 'no_provider', layer: 'auth', retryable: false }
  }

  const status = PROVIDER_CONTEXT_RE.test(text) ? providerErrorStatus(text) : null

  if (status === 401) {
    return { code: 'provider_key_rejected', layer: 'auth', retryable: false }
  }

  if (status === 404) {
    return {
      code: isLocalEndpointError(text) ? 'provider_model_not_installed' : 'provider_model_missing',
      layer: 'provider',
      retryable: false
    }
  }

  if (status === 429) {
    return { code: 'provider_rate_limited', layer: 'provider', retryable: true }
  }

  if (status !== null && status >= 500) {
    return { code: 'provider_server_error', layer: 'provider', retryable: true }
  }

  return null
}

/** A raw provider dump (endpoint block, response body, HTML) that must not be a toast's main line. */
export function isRawProviderDump(message: null | string | undefined): boolean {
  const text = message ?? ''

  return /<\s*(?:!doctype|html|body|head)\b/i.test(text) || /^\s*(?:endpoint|response|status):/im.test(text)
}

/** Details text: one copy of a repeated "retried after reloading auth" error, and no local-server hint
 *  where it cannot apply (a remote provider, or a rejected key). */
export function tidyErrorDetails(raw: null | string | undefined): string {
  let text = (raw ?? '').trim()

  if (text.includes(RETRIED_MARK)) {
    // "X (retried after reloading auth from disk: X)", possibly nested: keep X once when every
    // attempt failed the same way; keep the whole text when the retry said something different.
    const attempts = text.split(RETRIED_MARK).map(part => part.replace(/\)+\s*$/, '').trim())

    if (attempts.every(attempt => attempt === attempts[0])) {
      text = text.split(RETRIED_MARK)[0].trim()
    }
  }

  if (providerErrorStatus(text) === 401 || !isLocalEndpointError(text)) {
    text = text
      .replace(ENDPOINT_HINT_LINE_RE, '')
      .replace(/\n{2,}/g, '\n')
      .trim()
  }

  return text
}
