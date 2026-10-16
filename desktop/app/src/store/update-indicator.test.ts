import { describe, expect, it } from 'vitest'

import type { DesktopUpdateStatus } from '@/global'

import { updateIndicatorState } from './update-indicator'

const available: DesktopUpdateStatus = {
  supported: true,
  updateAvailable: true,
  latestVersion: 'v0.0.10',
  targetSha: 'abc123'
}

describe('updateIndicatorState', () => {
  it('shows with the version when an update is available', () => {
    expect(updateIndicatorState(available)).toEqual({
      downloadable: false,
      kind: null,
      phase: 'available',
      visible: true,
      version: '0.0.10'
    })
  })

  it('shows without a version when the feed names none but a target exists', () => {
    expect(updateIndicatorState({ supported: true, behind: 3, targetSha: 'abc' })).toEqual({
      downloadable: false,
      kind: null,
      phase: 'available',
      visible: true,
      version: null
    })
  })

  it('is hidden when there is no update, no status, or a stale zero count', () => {
    expect(updateIndicatorState(null).visible).toBe(false)
    expect(updateIndicatorState({ supported: true, updateAvailable: false, targetSha: 'x' }).visible).toBe(false)
    expect(updateIndicatorState({ supported: true, behind: 0, targetSha: 'x' }).visible).toBe(false)
  })

  it('is hidden when the check errored, with no error dot', () => {
    expect(updateIndicatorState({ ...available, error: 'check-failed' }).visible).toBe(false)
  })

  it('is hidden where the check is unsupported (remote or gateway-only contexts)', () => {
    expect(updateIndicatorState({ ...available, supported: false }).visible).toBe(false)
  })

  it('is hidden while onboarding is active and while offline', () => {
    expect(updateIndicatorState(available, { onboarding: true }).visible).toBe(false)
    expect(updateIndicatorState(available, { online: false }).visible).toBe(false)
  })

  it('does not look at the toast snooze: a dismissed toast still leaves the dot', () => {
    window.localStorage.setItem('factr:update-toast-snooze-until', String(Date.now() + 86_400_000))

    expect(updateIndicatorState(available).visible).toBe(true)
  })

  it('carries the kind of the ready file (zip means restart to update)', () => {
    const download = { status: 'ready' as const, version: '0.0.10', file: '/x.zip', kind: 'zip' as const }

    expect(updateIndicatorState(available, { download })).toMatchObject({ kind: 'zip', phase: 'ready' })
  })

  it('moves to downloading and ready only for the offered version', () => {
    const base = { version: '0.0.10', file: null, kind: null }

    expect(updateIndicatorState(available, { download: { ...base, status: 'downloading' } }).phase).toBe('downloading')
    expect(updateIndicatorState(available, { download: { ...base, status: 'ready', file: '/x.dmg', kind: 'dmg' as const } }).phase).toBe('ready')
    expect(updateIndicatorState(available, { download: { status: 'ready', version: '0.0.9', file: '/x', kind: 'dmg' } }).phase).toBe(
      'available'
    )
    expect(updateIndicatorState(available, { download: { ...base, status: 'failed-silent' } }).phase).toBe('available')
  })

  it('follows the download state whatever the auto-download setting (a manual download shows too)', () => {
    const download = { status: 'downloading' as const, version: '0.0.10', file: null, kind: null }

    expect(updateIndicatorState(available, { download }).phase).toBe('downloading')
  })

  it('knows whether a download is possible from the feed asset', () => {
    const asset = { kind: 'zip', name: 'a.zip', sha256: 'ab', size: 1, url: 'https://github.com/x' }

    expect(updateIndicatorState({ ...available, asset }).downloadable).toBe(true)
    expect(updateIndicatorState(available).downloadable).toBe(false)
  })
})
