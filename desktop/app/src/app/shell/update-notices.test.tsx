import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/store/updates', async () => {
  const { atom } = await import('nanostores')

  return { $desktopVersion: atom(null), $updateStatus: atom(null), openUpdatesWindow: vi.fn() }
})

vi.mock('@/store/session-states', async () => {
  const { atom } = await import('nanostores')

  return { $workingSessionIds: atom<string[]>([]) }
})

vi.mock('@/store/update-download', async importOriginal => {
  const actual = (await importOriginal()) as Record<string, unknown>

  return { ...actual, installDownloadedUpdate: vi.fn(), startDownloadNow: vi.fn() }
})

vi.mock('@/components/ui/button', () => ({
  Button: ({ children, onClick }: { children: React.ReactNode; onClick?: () => void }) => (
    <button onClick={onClick} type="button">
      {children}
    </button>
  )
}))

vi.mock('@/i18n', () => ({
  useI18n: () => ({
    t: {
      common: { close: 'Close' },
      notifications: {
        updateBannerReady: (v: string) => `Factr-I ${v} is ready. Restart to update.`,
        updateBannerRestart: 'Restart',
        updatedTo: (v: string) => `Updated to ${v}`
      }
    }
  })
}))

import { setOnboardingSurfaceActive } from '@/store/onboarding-presence'
import { $updateDownload, installDownloadedUpdate } from '@/store/update-download'
import { $updateBannerDismissed, $updatedNote } from '@/store/update-notices'

import { UpdateNotices } from './update-notices'

const setDownload = (value: unknown) => ($updateDownload as unknown as { set: (v: unknown) => void }).set(value)
const ready = { status: 'ready', version: '0.0.10', file: '/u/a.zip', kind: 'zip' }

describe('UpdateNotices', () => {
  beforeEach(() => {
    setDownload({ status: 'idle', version: null, file: null, kind: null })
    $updateBannerDismissed.set(false)
    $updatedNote.set(null)
    setOnboardingSurfaceActive('intro', false)
    vi.mocked(installDownloadedUpdate).mockClear()
  })

  afterEach(cleanup)

  it('shows nothing without a ready update', () => {
    const { container } = render(<UpdateNotices />)

    expect(container.firstChild).toBeNull()
  })

  it('shows the restart banner for a ready zip and Restart installs', () => {
    setDownload(ready)
    render(<UpdateNotices />)

    expect(screen.getByText('Factr-I 0.0.10 is ready. Restart to update.')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Restart' }))
    expect(installDownloadedUpdate).toHaveBeenCalledTimes(1)
  })

  it('closes for this launch and stays hidden during onboarding', () => {
    setDownload(ready)
    const { container, rerender } = render(<UpdateNotices />)

    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    rerender(<UpdateNotices />)
    expect(container.firstChild).toBeNull()

    $updateBannerDismissed.set(false)
    setOnboardingSurfaceActive('intro', true)
    rerender(<UpdateNotices />)
    expect(container.firstChild).toBeNull()
  })

  it('shows the Updated to note and clears it on close', () => {
    window.localStorage.removeItem('factr:updated-note-seen')
    $updatedNote.set({ from: '0.0.9', version: '0.0.10' })
    render(<UpdateNotices />)

    expect(screen.getByText('Updated to 0.0.10')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect($updatedNote.get()).toBeNull()
  })
})
