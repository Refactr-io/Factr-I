import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DesktopUpdateStatus } from '@/global'

vi.mock('@/store/updates', async () => {
  const { atom } = await import('nanostores')

  return { $updateStatus: atom(null), openUpdatesWindow: vi.fn() }
})

vi.mock('@/components/ui/tooltip', () => ({ Tip: ({ children }: { children: React.ReactNode }) => children }))

vi.mock('@/i18n', () => ({
  useI18n: () => ({
    t: {
      notifications: {
        updateAvailableTip: (version: null | string) =>
          version ? `Update available: Factr-I v${version}` : 'Update available'
      }
    }
  })
}))

import { setOnboardingSurfaceActive } from '@/store/onboarding-presence'
import { $updateStatus, openUpdatesWindow } from '@/store/updates'

import { UpdateIndicator } from './update-indicator'

const setStatus = (value: DesktopUpdateStatus | null) =>
  ($updateStatus as unknown as { set: (v: DesktopUpdateStatus | null) => void }).set(value)

const available: DesktopUpdateStatus = {
  supported: true,
  updateAvailable: true,
  latestVersion: '0.0.10',
  targetSha: 'abc'
}

describe('UpdateIndicator', () => {
  beforeEach(() => {
    setStatus(null)
    setOnboardingSurfaceActive('intro', false)
    vi.mocked(openUpdatesWindow).mockClear()
  })

  afterEach(cleanup)

  it('renders nothing without an update', () => {
    const { container } = render(<UpdateIndicator />)

    expect(container.firstChild).toBeNull()
  })

  it('shows the version label and opens the client updates overlay on click', () => {
    setStatus(available)
    render(<UpdateIndicator />)

    fireEvent.click(screen.getByRole('button', { name: 'Update available: Factr-I v0.0.10' }))

    expect(openUpdatesWindow).toHaveBeenCalledWith('client')
  })

  it('stays visible after the toast was snoozed or dismissed', () => {
    window.localStorage.setItem('factr:update-toast-snooze-until', String(Date.now() + 86_400_000))
    setStatus(available)
    render(<UpdateIndicator />)

    expect(screen.getByRole('button', { name: /Update available/ })).toBeTruthy()
  })

  it('hides on a failed check, an unsupported install and during onboarding', () => {
    setStatus({ ...available, error: 'check-failed' })
    const { container, rerender } = render(<UpdateIndicator />)

    expect(container.firstChild).toBeNull()

    setStatus({ ...available, supported: false })
    rerender(<UpdateIndicator />)
    expect(container.firstChild).toBeNull()

    setStatus(available)
    setOnboardingSurfaceActive('intro', true)
    rerender(<UpdateIndicator />)
    expect(container.firstChild).toBeNull()
  })
})
