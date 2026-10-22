import { afterEach, describe, expect, it } from 'vitest'

import { TRANSLATIONS } from '@/i18n'

import { errorRecoveryPlan } from './error-surface'
import { errorCardText } from './error-surface-copy'
import {
  inferErrorSurface,
  isRawProviderDump,
  isUnsupportedModelError,
  noteSavedProvider,
  noUsableCredentialsMessage,
  providerErrorStatus,
  tidyErrorDetails
} from './provider-error-display'

const thread = TRANSLATIONS.en.assistant.thread

const dump = (endpoint: string, status: string, hint: string) =>
  [
    'OpenAI-compatible chat request failed',
    `  endpoint: ${endpoint}`,
    '  model: stub-model',
    '  auth: api key',
    `  status: ${status}`,
    '  response: {"error":{"message":"x"}}',
    hint
  ].join('\n')

const LOCAL_HINT =
  'Local endpoint hint: make sure the server is running, the base URL includes /v1, the selected model is loaded, and the server supports streaming POST /chat/completions.'

const REMOTE_HINT = 'Hint: check network connectivity, DNS/TLS, that the base URL includes the API version (usually /v1).'

const title = (raw: string) => errorCardText(thread, inferErrorSurface(raw)).title
const body = (raw: string) => errorCardText(thread, inferErrorSurface(raw)).body

describe('no provider configured', () => {
  const raw =
    'Claude credentials not available. Run `factr login claude` first. (retried after reloading auth from disk: Claude credentials not available. Run `factr login claude` first.)'

  it('reads as a plain "connect a provider" card with a Providers link and no Retry', () => {
    expect(title(raw)).toBe('No AI provider is connected yet')
    expect(body(raw)).toBe('Open Settings > Providers (or sign in) to choose one.')
    const plan = errorRecoveryPlan(inferErrorSurface(raw))
    expect(plan.openProviders).toBe(true)
    expect(plan.retry).toBe(false)
  })

  it('collapses the repeated "retried after reloading" text in Details', () => {
    expect(tidyErrorDetails(raw)).toBe('Claude credentials not available. Run `factr login claude` first.')
  })

  it('keeps a retry that failed differently', () => {
    const different = 'A failed (retried after reloading auth from disk: B failed)'
    expect(tidyErrorDetails(different)).toBe(different)
  })
})

describe('provider HTTP status', () => {
  it('401: the provider rejected the key or login, without the local-endpoint hint', () => {
    const raw = dump('http://127.0.0.1:8080/v1/chat/completions', '401 Unauthorized', LOCAL_HINT)
    expect(title(raw)).toBe('The AI service rejected the key or sign-in')
    expect(tidyErrorDetails(raw)).not.toMatch(/Local endpoint hint/)
    expect(tidyErrorDetails(raw)).toContain('status: 401 Unauthorized')
  })

  it('404 on a remote provider: the provider does not have this model', () => {
    const raw = dump('https://api.example.com/v1/chat/completions', '404 Not Found', REMOTE_HINT)
    expect(title(raw)).toBe('The provider does not have this model')
    expect(body(raw)).not.toMatch(/install|ollama/i)
    expect(errorRecoveryPlan(inferErrorSurface(raw)).chooseModel).toBe(true)
    expect(tidyErrorDetails(raw)).not.toMatch(/Hint:/)
  })

  it('404 on a local endpoint keeps a short "is the model installed" hint', () => {
    const raw = dump('http://localhost:11434/v1/chat/completions', '404 Not Found', LOCAL_HINT)
    expect(title(raw)).toBe('The provider does not have this model')
    expect(body(raw)).toMatch(/installed on your local server/)
    expect(tidyErrorDetails(raw)).toContain('Local endpoint hint')
  })

  it('429: rate limited, retrying', () => {
    expect(title(dump('https://api.example.com/v1', '429 Too Many Requests', ''))).toBe('Rate limited. Try again in a moment.')
  })

  it.each(['500 Internal Server Error', '502 Bad Gateway', '503 Service Unavailable'])('%s: a server error', status => {
    const raw = dump('https://api.example.com/v1', status, '')
    expect(title(raw)).toBe('The provider had a server error.')
    expect(errorRecoveryPlan(inferErrorSurface(raw)).retry).toBe(true)
  })

  it.each([
    'OpenAI rejected the access token and no refresh token is available; run `factr login` to re-authenticate: {"error":{}}',
    'OpenAI request failed: 401 unauthorized',
    'OpenAI error: Incorrect API key provided: sk-qa***',
    'Anthropic: invalid api key'
  ])('reads a rejected key from engine text: %s', raw => {
    const surface = inferErrorSurface(raw)
    expect(surface?.code).toBe('provider_key_rejected')
    const { body, title } = errorCardText(TRANSLATIONS.en.assistant.thread, surface)
    expect(title).toBe('The AI service rejected the key or sign-in')
    expect(body).toMatch(/Replace it in Settings > Providers/)
    expect(body).not.toMatch(/factr login/)
    expect(tidyErrorDetails(raw)).toContain(raw.split(':')[0])
  })

  it.each([
    'OpenAI stream closed before the response completed',
    'unexpected end of stream',
    'malformed SSE event from provider',
    'The model ended its turn without any visible output (stop_reason: unknown)',
    'response truncated'
  ])('reads a cut-off stream: %s', raw => {
    const surface = inferErrorSurface(raw)
    expect(surface?.code).toBe('truncated')
    expect(errorCardText(TRANSLATIONS.en.assistant.thread, surface).body).toBe('The reply was cut off. Retry to continue.')
  })

  it('reads other status spellings', () => {
    expect(providerErrorStatus('Anthropic API error (401 Unauthorized): invalid x-api-key')).toBe(401)
    expect(providerErrorStatus('model request: HTTP 429')).toBe(429)
  })

  it('does not read a status from text that is not about a provider call', () => {
    expect(inferErrorSurface('HTTP 404: session not found')).toBeNull()
    expect(inferErrorSurface('Something unrelated happened')).toBeNull()
  })
})

describe('raw dumps', () => {
  it('flags response bodies and HTML, not plain sentences', () => {
    expect(isRawProviderDump('x\n  response: {"a":1}')).toBe(true)
    expect(isRawProviderDump('<html>Internal Server Error upstream</html>')).toBe(true)
    expect(isRawProviderDump('Factr could not start the assistant for this chat.')).toBe(false)
  })
})

describe('noUsableCredentialsMessage', () => {
  it('does not name a provider the user never chose', () => {
    for (const raw of ['No usable credentials found for Claude.', 'No usable credentials found for openai-compatible.']) {
      expect(noUsableCredentialsMessage(raw, null)).toBe(
        'No AI provider is connected yet. Open Settings > Providers (or sign in) to choose one.'
      )
    }
  })

  it('names the saved provider when there is one', () => {
    expect(noUsableCredentialsMessage('No usable credentials found for huggingface.', 'huggingface')).toBe(
      'Your saved provider huggingface has no usable credentials. Open Settings > Providers to sign in again or choose another.'
    )
    expect(
      noUsableCredentialsMessage(
        'No usable credentials found for openai. setup.status reports configured credentials, but runtime resolution still failed.',
        'openai-codex'
      )
    ).toContain('Your saved provider openai has no usable credentials.')
  })

  it('leaves other text, and engines that do not report the saved provider, alone', () => {
    expect(noUsableCredentialsMessage('The custom endpoint https://h/v1 needs an API key and none was found.', null)).toBeNull()
    expect(noUsableCredentialsMessage('No usable credentials found for Claude.', undefined)).toBeNull()
    expect(noUsableCredentialsMessage(null, null)).toBeNull()
  })
})

describe('a saved provider whose model or credentials the engine could not use', () => {
  const noCreds = 'OpenAI credentials not available. Run `factr login openai` first.'

  const unsupported =
    "Unsupported OpenAI model 'gpt-6-astra'. Use /model to choose from the models available to your account."

  afterEach(() => noteSavedProvider(undefined))

  it('says "no AI provider" only when nothing is saved (or the engine does not say)', () => {
    noteSavedProvider(null)
    expect(title(noCreds)).toBe('No AI provider is connected yet')
    noteSavedProvider(undefined)
    expect(title(noCreds)).toBe('No AI provider is connected yet')
    noteSavedProvider('  ')
    expect(title(noCreds)).toBe('No AI provider is connected yet')
  })

  it('does not claim nothing is connected when a provider is saved', () => {
    noteSavedProvider('openai-api')
    expect(title(noCreds)).toBe('Your saved provider could not be used')
    expect(body(noCreds)).toContain('Check its key and the selected model in Settings > Providers')
    const plan = errorRecoveryPlan(inferErrorSurface(noCreds))
    expect(plan.openProviders).toBe(true)
    expect(plan.chooseModel).toBe(true)
    expect(plan.retry).toBe(false)
  })

  it('reads an unsupported saved model as a model problem, with or without a saved provider', () => {
    for (const saved of [null, undefined, 'openai-api']) {
      noteSavedProvider(saved)
      expect(title(unsupported)).toBe('The provider does not have this model')
      expect(body(unsupported)).toBe('Choose another model, then send your message again.')
      expect(errorRecoveryPlan(inferErrorSurface(unsupported)).chooseModel).toBe(true)
    }

    noteSavedProvider(null)
    expect(title(`${unsupported} ${noCreds}`)).toBe('The provider does not have this model')
  })

  it('reads the engine\'s real "saved model failed and so did the startup model" text as a model problem', () => {
    // From the O-01c QA run: a saved OpenAI key, a saved default the account list lacks.
    const raw =
      "the saved model gpt-6-astra failed (Unsupported OpenAI model 'gpt-6-astra'. Use /model to choose from the models available to your account.) and so did the startup model claude-opus-5-5: Claude credentials not available. Run `factr login claude` first."

    noteSavedProvider('openai-api')
    expect(title(raw)).toBe('The provider does not have this model')
    expect(tidyErrorDetails(raw)).toContain("Unsupported OpenAI model 'gpt-6-astra'")
  })

  it('recognises the unsupported-model text only', () => {
    expect(isUnsupportedModelError(unsupported)).toBe(true)
    expect(isUnsupportedModelError('Unsupported model foo')).toBe(true)
    expect(isUnsupportedModelError(noCreds)).toBe(false)
    expect(isUnsupportedModelError(undefined)).toBe(false)
  })
})

describe('budget stop', () => {
  const raw =
    'Budget reached: this session has used 18 tokens of its 15 token limit (budget.max_tokens). Raise or clear the limit in Settings to continue.'

  it('reads the engine budget message as its own, non-retryable failure', () => {
    const surface = inferErrorSurface(raw)

    expect(surface).toMatchObject({ code: 'budget_reached', retryable: false })
    expect(errorRecoveryPlan(surface).retry).toBe(false)
    expect(errorCardText(thread, surface).title).toBe('Session budget reached')
  })
})
