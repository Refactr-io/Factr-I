import { afterEach, describe, expect, it } from 'vitest'

import { inferErrorSurface, noteSavedProvider } from './provider-error-display'
import { fetchRuntimeReadinessSignals, interpretRuntimeReadiness, runtimeReadinessDisplay } from './runtime-readiness'

describe('interpretRuntimeReadiness', () => {
  // D-3: a corrupt config.yaml must be named, not hidden behind "No usable credentials".
  it('names an unreadable config.yaml instead of asking for credentials', () => {
    const configError = {
      line: 4,
      message: '/x/.factr/config.yaml could not be read (line 4), so Factr-I is ignoring it: did not find expected node.',
      path: '/x/.factr/config.yaml'
    }

    const result = interpretRuntimeReadiness({
      setup: { config_error: configError, provider_configured: false, saved_provider: null },
      setupError: null,
      runtime: { config_error: configError, error: configError.message, ok: false, provider: 'openrouter' },
      runtimeError: null
    })

    expect(result.ready).toBe(false)
    expect(result.configError).toEqual(configError)
    expect(result.reason).toBe(configError.message)
    expect(result.reason).not.toMatch(/credentials/i)
  })

  it('prefers runtime_check when both signals exist', () => {
    const result = interpretRuntimeReadiness({
      setup: { provider_configured: false },
      setupError: null,
      runtime: { ok: true },
      runtimeError: null
    })

    expect(result).toEqual({
      checksDisagree: true,
      ready: true,
      reason: null,
      source: 'runtime_check'
    })
  })

  it('surfaces runtime mismatch details when runtime_check fails', () => {
    const result = interpretRuntimeReadiness({
      setup: { provider_configured: true },
      setupError: null,
      runtime: { error: 'No provider can serve the selected model.', ok: false },
      runtimeError: null
    })

    expect(result.ready).toBe(false)
    expect(result.source).toBe('runtime_check')
    expect(result.checksDisagree).toBe(true)
    expect(result.reason).toContain('No provider can serve the selected model.')
    expect(result.reason).toContain('setup.status reports configured credentials')
  })

  it('falls back to setup.status when runtime_check has no boolean result', () => {
    const result = interpretRuntimeReadiness({
      setup: { provider_configured: true },
      setupError: null,
      runtime: null,
      runtimeError: 'runtime check RPC unavailable'
    })

    expect(result).toEqual({
      checksDisagree: false,
      ready: true,
      reason: null,
      source: 'setup_status'
    })
  })

  it('uses explicit fallback when both checks are missing', () => {
    const result = interpretRuntimeReadiness({
      setup: null,
      setupError: 'setup.status timeout',
      runtime: null,
      runtimeError: 'setup.runtime_check timeout'
    })

    expect(result.ready).toBe(false)
    expect(result.source).toBe('fallback')
    expect(result.reason).toBe('setup.runtime_check timeout')
  })
})

describe('fetchRuntimeReadinessSignals', () => {
  it('scopes setup.runtime_check to the requested provider', async () => {
    const calls: Array<{ method: string; params?: Record<string, unknown> }> = []

    const requestGateway = async <T = unknown>(method: string, params?: Record<string, unknown>) => {
      calls.push({ method, params })

      if (method === 'setup.status') {
        return { provider_configured: true } as T
      }

      if (method === 'setup.runtime_check') {
        return { ok: true } as T
      }

      throw new Error(`unexpected method: ${method}`)
    }

    await fetchRuntimeReadinessSignals(requestGateway, 'openrouter')

    expect(calls).toEqual([
      { method: 'setup.status' },
      { method: 'setup.runtime_check', params: { provider: 'openrouter' } }
    ])
  })

  describe('the saved provider it learns for plain error wording', () => {
    const noCreds = 'OpenAI credentials not available. Run `factr login openai` first.'

    const statusWith = (saved: Record<string, unknown>) => async <T = unknown>(method: string) => {
      if (method === 'setup.status') {
        return { provider_configured: true, ...saved } as T
      }

      return { ok: false } as T
    }

    afterEach(() => noteSavedProvider(undefined))

    it('a saved provider turns "credentials not available" into "your saved provider could not be used"', async () => {
      await fetchRuntimeReadinessSignals(statusWith({ saved_provider: 'openai-api' }))
      expect(inferErrorSurface(noCreds)?.code).toBe('provider_unusable')
      await fetchRuntimeReadinessSignals(statusWith({ saved_provider: null }))
      expect(inferErrorSurface(noCreds)?.code).toBe('no_provider')
    })
  })
})

describe('runtimeReadinessDisplay', () => {
  it('does not call configured credentials setup when runtime resolution fails', () => {
    expect(
      runtimeReadinessDisplay({
        checksDisagree: true,
        ready: false,
        reason: 'Anthropic cannot serve the selected model.',
        source: 'runtime_check'
      })
    ).toBe('unavailable')
  })

  it('keeps needs-setup for an authoritative unconfigured result', () => {
    expect(
      runtimeReadinessDisplay({
        checksDisagree: false,
        ready: false,
        reason: 'No provider configured.',
        source: 'setup_status'
      })
    ).toBe('needs_setup')
  })
})

describe('interpretRuntimeReadiness: the engine\'s "No usable credentials" text', () => {
  const failing = (saved: null | string | undefined) =>
    interpretRuntimeReadiness({
      setup: { provider_configured: false, ...(saved === undefined ? {} : { saved_provider: saved }) },
      setupError: null,
      runtime: { ok: false, error: 'No usable credentials found for Claude.', provider: 'Claude' },
      runtimeError: null
    })

  it('reads as "nothing connected yet" on a first run, with the raw text kept as the detail', () => {
    const result = failing(null)

    expect(result.ready).toBe(false)
    expect(result.reason).toBe('No AI provider is connected yet. Open Settings > Providers (or sign in) to choose one.')
    expect(result.reasonDetail).toBe('No usable credentials found for Claude.')
  })

  it('names the saved provider when one is saved', () => {
    expect(failing('anthropic').reason).toBe(
      'Your saved provider Claude has no usable credentials. Open Settings > Providers to sign in again or choose another.'
    )
  })

  it('keeps the raw text when the engine does not say what is saved', () => {
    expect(failing(undefined)).toMatchObject({ reason: 'No usable credentials found for Claude.' })
    expect(failing(undefined).reasonDetail).toBeUndefined()
  })
})
