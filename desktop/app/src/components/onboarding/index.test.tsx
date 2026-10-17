import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { $desktopOnboarding, type DesktopOnboardingState, type OnboardingContext } from '@/store/onboarding'
import { makeOAuthProvider } from '@/test/oauth-provider'
import type { OAuthProvider } from '@/types/factr'

import { BootErrorNotice, Picker, ReasonNotice, useEscapeClosesManualOnboarding } from '.'

function setProviders(providers: OAuthProvider[]) {
  $desktopOnboarding.set({
    configured: false,
    flow: { status: 'idle' },
    mode: 'oauth',
    providers,
    reason: null,
    requested: false,
    firstRunSkipped: false,
    manual: false,
    localEndpoint: false
  } satisfies DesktopOnboardingState)
}

const ctx: OnboardingContext = { requestGateway: async () => undefined as never }

afterEach(() => {
  cleanup()

  try {
    window.localStorage.clear()
  } catch {
    // jsdom localStorage should always be present; ignore if not.
  }

  $desktopOnboarding.set({
    configured: null,
    flow: { status: 'idle' },
    mode: 'oauth',
    providers: null,
    reason: null,
    requested: false,
    firstRunSkipped: false,
    manual: false,
    localEndpoint: false
  })
})

describe('onboarding Picker', () => {
  it('features ChatGPT and hides other providers behind a disclosure', () => {
    setProviders([
      makeOAuthProvider('anthropic', 'Anthropic Claude'),
      makeOAuthProvider('openai-codex', 'OpenAI Codex / ChatGPT')
    ])
    render(<Picker ctx={ctx} />)

    expect(screen.getByText('ChatGPT plan')).toBeTruthy()
    expect(screen.getByText('Recommended')).toBeTruthy()
    // Fireworks stays behind the disclosure with the other alternatives; only
    // ChatGPT is visible before the user expands the list.
    expect(screen.queryByText('Fireworks AI')).toBeNull()
    expect(screen.queryByText('Anthropic API Key')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Other providers' }))

    expect(screen.getByText('Fireworks AI')).toBeTruthy()
    expect(screen.getByText('Anthropic API Key')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Collapse' })).toBeTruthy()
  })

  it('puts ChatGPT first with no Fireworks preselected when only the fallback list is known', () => {
    setProviders([makeOAuthProvider('openai-codex', 'ChatGPT plan')])
    render(<Picker ctx={ctx} />)

    expect(screen.getByText('ChatGPT plan')).toBeTruthy()
    expect(screen.queryByText('Fireworks AI')).toBeNull()
    expect(screen.queryByRole('textbox')).toBeNull()
  })

  it('shows every provider directly when ChatGPT is absent', () => {
    setProviders([makeOAuthProvider('anthropic', 'Anthropic Claude')])
    render(<Picker ctx={ctx} />)

    expect(screen.getByText('Fireworks AI')).toBeTruthy()
    expect(screen.getByText('Anthropic API Key')).toBeTruthy()
    expect(screen.queryByText('Other sign-in options')).toBeNull()
    expect(screen.queryByText('Recommended')).toBeNull()
  })

  it('offers "choose later" on first run and persists the skip', () => {
    setProviders([makeOAuthProvider('openai-codex', 'OpenAI Codex / ChatGPT')])
    render(<Picker ctx={ctx} />)

    const skip = screen.getByRole('button', { name: "I'll choose a provider later" })

    fireEvent.click(skip)

    expect($desktopOnboarding.get().firstRunSkipped).toBe(true)
    expect(window.localStorage.getItem('factr-onboarding-skipped-v1')).toBe('1')
  })

  it('hides "choose later" in manual (add-provider) mode', () => {
    setProviders([makeOAuthProvider('openai-codex', 'OpenAI Codex / ChatGPT')])
    $desktopOnboarding.set({ ...$desktopOnboarding.get(), manual: true })
    render(<Picker ctx={ctx} />)

    expect(screen.queryByRole('button', { name: "I'll choose a provider later" })).toBeNull()
  })
})

describe('Esc on the provider selector', () => {
  it('closes a manually opened selector', () => {
    setProviders([makeOAuthProvider('anthropic', 'Anthropic Claude')])
    $desktopOnboarding.set({ ...$desktopOnboarding.get(), manual: true })
    renderHook(() => useEscapeClosesManualOnboarding(true))

    fireEvent.keyDown(window.document, { key: 'Escape' })

    expect($desktopOnboarding.get().manual).toBe(false)
  })

  it('leaves first-run onboarding open (no close affordance there)', () => {
    setProviders([makeOAuthProvider('anthropic', 'Anthropic Claude')])
    renderHook(() => useEscapeClosesManualOnboarding(false))

    fireEvent.keyDown(window.document, { key: 'Escape' })

    expect($desktopOnboarding.get().configured).toBe(false)
  })
})

// QA D-7: the disk-full start failure showed the raw IPC text next to the friendly card.
describe('BootErrorNotice', () => {
  it('leads with the plain cause and keeps the raw text under Details', () => {
    const raw =
      "Error invoking remote method 'factr:connection': Error: Factr-I backend: exited before port announcement (1) " +
      'Recent backend output: Error: opening factr.db Caused by: 1: database or disk is full'

    render(<BootErrorNotice error={raw} />)

    expect(screen.getByText('The disk is full, so Factr-I could not start.')).toBeTruthy()
    const details = screen.getByText(raw).closest('details')
    expect(details).not.toBeNull()
    expect(details?.open).toBe(false)
  })
})

// QA D-3: an unreadable config.yaml is named, with Open the folder / Back up and start fresh.
describe('ReasonNotice for an unreadable config.yaml', () => {
  const configError = {
    line: 4,
    message: '/x/.factr/config.yaml could not be read (line 4), so Factr-I is ignoring it: bad yaml.',
    path: '/x/.factr/config.yaml'
  }

  afterEach(() => {
    delete (window as { factrDesktop?: unknown }).factrDesktop
  })

  it('reveals the file and backs it up before anything else', async () => {
    const revealPath = vi.fn(async () => true)

    const backupCorruptConfig = vi.fn(async () => ({ ok: true as const, backupPath: `${configError.path}.corrupt-1` }))

    ;(window as { factrDesktop?: unknown }).factrDesktop = { backupCorruptConfig, revealPath }
    setProviders([])

    render(<ReasonNotice configError={configError} ctx={ctx} reason={configError.message} />)

    expect(screen.getByText(configError.message)).toBeTruthy()
    fireEvent.click(screen.getByText('Open the folder'))
    expect(revealPath).toHaveBeenCalledWith(configError.path)

    await act(async () => {
      fireEvent.click(screen.getByText('Back up and start fresh'))
    })

    expect(backupCorruptConfig).toHaveBeenCalledWith(configError.path)
    expect(screen.getByText(/Saved a copy as \/x\/\.factr\/config\.yaml\.corrupt-1/)).toBeTruthy()
  })

  it('shows why a backup was refused and keeps the actions', async () => {
    const backupCorruptConfig = vi.fn(async () => ({ ok: false as const, message: 'not a regular file' }))

    ;(window as { factrDesktop?: unknown }).factrDesktop = { backupCorruptConfig }

    render(<ReasonNotice configError={configError} ctx={ctx} reason={configError.message} />)
    await act(async () => {
      fireEvent.click(screen.getByText('Back up and start fresh'))
    })

    expect(screen.getByText('not a regular file')).toBeTruthy()
    expect(screen.getByText('Back up and start fresh')).toBeTruthy()
  })
})
