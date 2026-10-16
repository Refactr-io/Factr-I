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
    expect(updateIndicatorState(available)).toEqual({ visible: true, version: '0.0.10' })
  })

  it('shows without a version when the feed names none but a target exists', () => {
    expect(updateIndicatorState({ supported: true, behind: 3, targetSha: 'abc' })).toEqual({
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
})
