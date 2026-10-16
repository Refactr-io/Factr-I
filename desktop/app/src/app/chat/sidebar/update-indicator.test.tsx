import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { DesktopUpdateStatus } from '@/global'

vi.mock('@/store/updates', async () => {
  const { atom } = await import('nanostores')

  return { $updateStatus: atom(null), openUpdatesWindow: vi.fn() }
})

vi.mock('@/store/update-download', async importOriginal => {
  const actual = (await importOriginal()) as Record<string, unknown>

  return { ...actual, installDownloadedUpdate: vi.fn(), startDownloadNow: vi.fn() }
})

vi.mock('@/store/session-states', async () => {
  const { atom } = await import('nanostores')

  return { $workingSessionIds: atom<string[]>([]) }
})

vi.mock('@/components/ui/tooltip', () => ({ Tip: ({ children }: { children: React.ReactNode }) => children }))

vi.mock('@/i18n', () => ({
  useI18n: () => ({
    t: {
      notifications: {
        updateAvailableTip: (version: null | string) =>
          version ? `Update available: Factr-I v${version}` : 'Update available',
        updateDownloadingTip: (version: string) => `Downloading Factr-I v${version}`,
        updateRestartTip: 'Restart to update',
        updateInstallTip: 'Install update',
        updateBusyTip: 'Finish the current task first'
      }
    }
  })
}))

import { setOnboardingSurfaceActive } from '@/store/onboarding-presence'
import { $workingSessionIds } from '@/store/session-states'
import { $autoDownload, $updateDownload, installDownloadedUpdate, startDownloadNow } from '@/store/update-download'
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
    vi.mocked(installDownloadedUpdate).mockClear()
    vi.mocked(startDownloadNow).mockClear()
    $autoDownload.set(true)
    ;($updateDownload as unknown as { set: (v: unknown) => void }).set({ status: 'idle', version: null, file: null, kind: null })
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

  it('shows a spinning loader while downloading and still opens the overlay on click', () => {
    setStatus(available)
    ;($updateDownload as unknown as { set: (v: unknown) => void }).set({ status: 'downloading', version: '0.0.10', file: null, kind: null })
    render(<UpdateIndicator />)

    const button = screen.getByRole('button', { name: 'Downloading Factr-I v0.0.10' })

    expect(button.getAttribute('data-phase')).toBe('downloading')
    expect(button.querySelector('.motion-safe\\:animate-\\[spin_1\\.6s_linear_infinite\\]')).not.toBeNull()
    fireEvent.click(button)
    expect(openUpdatesWindow).toHaveBeenCalledWith('client')
  })

  it('shows the ready state and installs on click instead of opening the overlay', () => {
    setStatus(available)
    ;($updateDownload as unknown as { set: (v: unknown) => void }).set({ status: 'ready', version: '0.0.10', file: '/x.dmg', kind: 'dmg' })
    render(<UpdateIndicator />)

    fireEvent.click(screen.getByRole('button', { name: 'Install update' }))

    expect(installDownloadedUpdate).toHaveBeenCalledTimes(1)
    expect(openUpdatesWindow).not.toHaveBeenCalled()
  })

  it('a ready macOS zip says Restart to update and installs on click', () => {
    setStatus(available)
    ;($updateDownload as unknown as { set: (v: unknown) => void }).set({ status: 'ready', version: '0.0.10', file: '/x.zip', kind: 'zip' })
    render(<UpdateIndicator />)

    fireEvent.click(screen.getByRole('button', { name: 'Restart to update' }))

    expect(installDownloadedUpdate).toHaveBeenCalledTimes(1)
  })

  it('is disabled with Finish the current task first while a turn runs', () => {
    setStatus(available)
    ;($updateDownload as unknown as { set: (v: unknown) => void }).set({ status: 'ready', version: '0.0.10', file: '/x.zip', kind: 'zip' })
    ;($workingSessionIds as unknown as { set: (v: string[]) => void }).set(['s1'])
    render(<UpdateIndicator />)

    const button = screen.getByRole('button', { name: 'Finish the current task first' }) as HTMLButtonElement

    expect(button.disabled).toBe(true)
    fireEvent.click(button)
    expect(installDownloadedUpdate).not.toHaveBeenCalled()
    ;($workingSessionIds as unknown as { set: (v: string[]) => void }).set([])
  })

  it('with auto-update off, an available update with an installer starts the download on click', () => {
    $autoDownload.set(false)
    setStatus({ ...available, asset: { kind: 'zip', name: 'a.zip', sha256: 'ab', size: 1, url: 'https://github.com/x' } })
    render(<UpdateIndicator />)

    fireEvent.click(screen.getByRole('button', { name: /Update available/ }))

    expect(startDownloadNow).toHaveBeenCalledTimes(1)
    expect(openUpdatesWindow).not.toHaveBeenCalled()
  })

  it('with auto-update on, or no installer listed, the click still opens the overlay', () => {
    setStatus({ ...available, asset: { kind: 'zip', name: 'a.zip', sha256: 'ab', size: 1, url: 'https://github.com/x' } })
    const { unmount } = render(<UpdateIndicator />)

    fireEvent.click(screen.getByRole('button', { name: /Update available/ }))
    expect(openUpdatesWindow).toHaveBeenCalledWith('client')
    expect(startDownloadNow).not.toHaveBeenCalled()
    unmount()

    $autoDownload.set(false)
    setStatus(available)
    render(<UpdateIndicator />)
    fireEvent.click(screen.getByRole('button', { name: /Update available/ }))
    expect(startDownloadNow).not.toHaveBeenCalled()
  })

  it('a ready Windows or Linux installer reads Install update', () => {
    setStatus(available)
    ;($updateDownload as unknown as { set: (v: unknown) => void }).set({ status: 'ready', version: '0.0.10', file: '/x.exe', kind: 'exe' })
    render(<UpdateIndicator />)

    fireEvent.click(screen.getByRole('button', { name: 'Install update' }))

    expect(installDownloadedUpdate).toHaveBeenCalledTimes(1)
  })
})
