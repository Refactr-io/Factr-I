// "Copy diagnostics": what a support request needs (system, version, provider and model, the last
// error, the settings) with every secret removed before it ever reaches the clipboard.

const REDACTED = '[redacted]'

// Keys whose value is a secret by name, wherever they sit in the config.
const SECRET_KEY =
  /(^|[_\-.])(api[_-]?key|key|token|secret|password|passwd|credential|credentials|auth|authorization|bearer|cookie|session[_-]?id|private|signature|salt|pin)($|[_\-.])|apikey|accesskey|secretkey|_key$|_token$/i

// Secret-looking text inside any string value.
const SECRET_TEXT: RegExp[] = [
  /\bsk-[A-Za-z0-9_-]{8,}/g,
  /\bgh[pousr]_[A-Za-z0-9]{20,}/g,
  /\bgithub_pat_[A-Za-z0-9_]{20,}/g,
  /\bxox[abprs]-[A-Za-z0-9-]{10,}/g,
  /\bAKIA[0-9A-Z]{16}\b/g,
  /\bAIza[0-9A-Za-z_-]{30,}/g,
  /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/g,
  /\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]{8,}/gi,
  /\b[0-9a-f]{40,}\b/gi
]

const URL_CREDENTIALS = /(\b[a-z][a-z0-9+.-]*:\/\/)[^/\s:@]+(?::[^/\s@]*)?@/gi
const URL_SECRET_QUERY = /([?&](?:api[_-]?key|key|token|access[_-]?token|secret|sig|signature|password)=)[^&\s#]+/gi

/** Redact secret-looking text in one string; also folds the user's home directory to `~`. */
export function redactText(text: string, home = ''): string {
  let out = text.replace(URL_CREDENTIALS, `$1${REDACTED}@`).replace(URL_SECRET_QUERY, `$1${REDACTED}`)

  for (const pattern of SECRET_TEXT) {
    out = out.replace(pattern, match =>
      /^(bearer|basic)\s/i.test(match) ? `${match.split(/\s/)[0]} ${REDACTED}` : REDACTED
    )
  }

  if (home.length > 1) {
    out = out.split(home).join('~')
  }

  // Home directories of any user, in any OS spelling, so a path never names the person.
  out = out
    .replace(/\/Users\/[^/\s'"]+/g, '~')
    .replace(/\/home\/[^/\s'"]+/g, '~')
    .replace(/[A-Za-z]:\\Users\\[^\\\s'"]+/g, '~')

  return out
}

/** A copy of `value` with secrets removed: by key name, by token shape and inside URLs. */
export function redactValue(value: unknown, home = '', key = '', depth = 0): unknown {
  if (depth > 12) {
    return REDACTED
  }

  if (typeof value === 'string') {
    return value && key && SECRET_KEY.test(key) ? REDACTED : redactText(value, home)
  }

  if (Array.isArray(value)) {
    // A list under a secret-named key (api_keys: [...]) is secret as a whole.
    return key && SECRET_KEY.test(key) ? REDACTED : value.map(item => redactValue(item, home, '', depth + 1))
  }

  if (value && typeof value === 'object') {
    // Environment and header maps carry secrets under arbitrary names: keep the names, hide every value.
    if (/^(env|environment|headers)$/i.test(key)) {
      return Object.fromEntries(Object.keys(value as Record<string, unknown>).map(name => [name, REDACTED]))
    }

    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([k, v]) => [k, redactValue(v, home, k, depth + 1)])
    )
  }

  // Numbers and booleans under a secret key (a PIN) are hidden too.
  return key && SECRET_KEY.test(key) && value !== null && value !== undefined ? REDACTED : value
}

export interface DiagnosticsInput {
  appVersion?: string
  backendVersion?: string
  config?: unknown
  home?: string
  lastError?: string
  locale?: string
  model?: string
  os?: string
  provider?: string
}

const MAX_CONFIG_CHARS = 20_000

/** The report: plain text, sections in the order a person triaging a problem reads them. */
export function buildDiagnostics(input: DiagnosticsInput): string {
  const home = input.home ?? ''
  const clean = (text: string | undefined) => redactText(text ?? '', home)
  let config = ''

  try {
    config = JSON.stringify(redactValue(input.config ?? {}, home), null, 2)
  } catch {
    config = '{}'
  }

  if (config.length > MAX_CONFIG_CHARS) {
    config = `${config.slice(0, MAX_CONFIG_CHARS)}\n... (cut)`
  }

  return [
    'Factr-I diagnostics',
    `App version: ${clean(input.appVersion) || 'unknown'}`,
    input.backendVersion ? `Backend version: ${clean(input.backendVersion)}` : '',
    `System: ${clean(input.os) || 'unknown'}`,
    input.locale ? `Language: ${clean(input.locale)}` : '',
    `Provider: ${clean(input.provider) || 'unknown'}`,
    `Model: ${clean(input.model) || 'unknown'}`,
    `Last error: ${clean(input.lastError) || 'none'}`,
    '',
    'Settings (secrets removed):',
    config
  ]
    .filter((line, index, all) => line !== '' || all[index - 1] !== '')
    .join('\n')
}
