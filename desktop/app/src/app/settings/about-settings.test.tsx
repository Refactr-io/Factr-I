// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { atom } from 'nanostores'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'

import { en } from '@/i18n/en'

const store = vi.hoisted(() => ({ checkUpdates: vi.fn(async () => null) }))

vi.mock('@/store/updates', async () => {
  const { atom } = await import('nanostores')

  return {
    $desktopVersion: atom({ appVersion: '0.1.0' }),
    $updateApply: atom({ applying: false, stage: 'idle' }),
    $updateChecking: atom(false),
    $updateStatus: atom(null),
    checkUpdates: store.checkUpdates,
    openUpdatesWindow: vi.fn(),
    refreshDesktopVersion: vi.fn(async () => null),
    startActiveUpdate: vi.fn()
  }
})
vi.mock('./uninstall-section', () => ({ UninstallSection: () => null }))

import * as updates from '@/store/updates'

import { AboutSettings } from './about-settings'

const openExternal = vi.fn(async () => undefined)

beforeEach(() => {
  Object.assign(window, { factrDesktop: { openExternal } })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  ;(updates.$updateStatus as ReturnType<typeof atom>).set(null)
})

const setStatus = (status: unknown) => (updates.$updateStatus as ReturnType<typeof atom>).set(status)

test('latest version is a neutral line with no error styling or docs reference', () => {
  setStatus({ supported: true, updateAvailable: false, currentVersion: '0.1.0', fetchedAt: Date.now() })
  const { container } = render(<AboutSettings />)

  const line = screen.getByText(en.settings.about.onLatest)
  expect(line).toBeTruthy()
  expect(container.innerHTML).not.toContain('text-destructive')
  expect(container.textContent).not.toContain('RELEASING')
  expect(screen.queryByText(en.settings.about.download)).toBeNull()
  expect(container.textContent).not.toContain('Branch')
})

test('a newer release shows the version and a Download button to the release page', () => {
  const releaseUrl = 'https://github.com/Refactr-io/Factr-I/releases/tag/v0.2.0'
  setStatus({
    supported: true,
    updateAvailable: true,
    currentVersion: '0.1.0',
    latestVersion: '0.2.0',
    releaseUrl,
    fetchedAt: Date.now()
  })
  render(<AboutSettings />)

  expect(screen.getByText('Version 0.2.0 is available.')).toBeTruthy()
  fireEvent.click(screen.getByText(en.settings.about.download))
  expect(openExternal).toHaveBeenCalledWith(releaseUrl)
  expect(screen.queryByText(en.settings.about.updateNow)).toBeNull()
})

test('offline check shows the unreachable notice', () => {
  setStatus({ supported: true, updateAvailable: false, currentVersion: '0.1.0', error: 'check-failed', message: 'net::ERR_PROXY_CONNECTION_FAILED' })
  render(<AboutSettings />)

  expect(screen.getByText("Couldn't check for updates right now.")).toBeTruthy()
  expect(screen.queryByText(/ERR_PROXY/)).toBeNull()
  // A failed check is neutral information, not a green "all good" tick.
  expect(screen.getByTestId('about-check-failed-icon')).toBeTruthy()
})
