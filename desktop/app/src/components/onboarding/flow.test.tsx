import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type * as FactrApi from '@/factr'
import { $desktopOnboarding, type DesktopOnboardingState, type OnboardingContext } from '@/store/onboarding'

import { FlowPanel } from './flow'

// Only the catalog fetch is replaced; the model assignment keeps its real path
// down to window.factrDesktop.api so the test observes the wire body.
vi.mock('@/factr', async importOriginal => ({
  ...(await importOriginal<typeof FactrApi>()),
  getGlobalModelOptions: async () => ({
    providers: [
      {
        models: ['gpt-5.6-terra'],
        name: 'OpenAI OAuth (ChatGPT)',
        pricing: { 'gpt-5.6-terra': { input: '$1.25', output: '$10.00' } },
        slug: 'openai'
      },
      { models: ['example/model-a'], name: 'OpenRouter', slug: 'openrouter' }
    ]
  })
}))

// The real picker is a cmdk/Radix dialog; stand in a button that reports the
// same {provider, model} selection shape the real one emits.
vi.mock('@/components/model-picker', () => ({
  ModelPickerDialog: ({
    onSelect,
    open
  }: {
    onSelect: (selection: { model: string; provider: string }) => void
    open: boolean
  }) =>
    open ? (
      <button onClick={() => onSelect({ model: 'example/model-a', provider: 'openrouter' })} type="button">
        pick-openrouter-model
      </button>
    ) : null
}))

const ctx: OnboardingContext = { requestGateway: async () => undefined as never }

function confirmingModelState(): DesktopOnboardingState {
  return {
    configured: false,
    flow: {
      status: 'confirming_model',
      currentModel: 'gpt-5.6-terra',
      label: 'OpenAI OAuth (ChatGPT)',
      providerSlug: 'openai',
      saving: false
    },
    mode: 'oauth',
    providers: null,
    reason: null,
    requested: false,
    firstRunSkipped: false,
    manual: false,
    localEndpoint: false
  }
}

function Harness() {
  const state = $desktopOnboarding.get()

  return (
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <FlowPanel ctx={ctx} flow={state.flow} leaving={false} onBegin={() => undefined} />
    </QueryClientProvider>
  )
}

beforeEach(() => {
  $desktopOnboarding.set(confirmingModelState())
})

afterEach(() => {
  cleanup()
  $desktopOnboarding.set({ ...confirmingModelState(), configured: null, flow: { status: 'idle' } })
})

describe('ConfirmingModelPanel model pick', () => {
  it('persists a cross-provider pick against the picked model provider, not the sign-in provider', async () => {
    const calls: { body?: unknown; path: string }[] = []

    Object.defineProperty(window, 'factrDesktop', {
      configurable: true,
      value: {
        api: async ({ body, path }: { body?: unknown; path: string }) => {
          calls.push({ body, path })

          if (path === '/api/model/set') {
            return { ok: true, provider: 'openrouter', model: 'example/model-a' }
          }

          throw new Error(`unexpected api path: ${path}`)
        }
      }
    })

    render(<Harness />)

    // The user signed in with OpenAI OAuth; the picker offers a model that
    // only OpenRouter serves.
    // Let the provider-catalog query settle so the relabel can read the picked provider's name.
    await new Promise(resolve => setTimeout(resolve, 200))
    fireEvent.click(screen.getByRole('button', { name: 'Change' }))
    fireEvent.click(await screen.findByRole('button', { name: 'pick-openrouter-model' }))

    await waitFor(() => expect(calls.some(c => c.path === '/api/model/set')).toBe(true))

    expect(calls.find(c => c.path === '/api/model/set')?.body).toMatchObject({
      scope: 'main',
      provider: 'openrouter',
      model: 'example/model-a'
    })

    // The relabel reads the picked provider's name from the catalog query.
    await waitFor(() => {
      const flow = $desktopOnboarding.get().flow
      expect(flow.status).toBe('confirming_model')

      if (flow.status === 'confirming_model') {
        expect(flow.providerSlug).toBe('openrouter')
        expect(flow.label).toBe('OpenRouter')
      }
    })
  })
})

describe('ConfirmingModelPanel price', () => {
  it('finds the price for a pick saved under the runtime id of the ChatGPT login', async () => {
    const state = confirmingModelState()

    if (state.flow.status === 'confirming_model') {
      $desktopOnboarding.set({ ...state, flow: { ...state.flow, providerSlug: 'openai-codex' } })
    }

    render(<Harness />)

    expect(await screen.findByText('$1.25 in / $10.00 out per Mtok')).toBeTruthy()
  })
})
