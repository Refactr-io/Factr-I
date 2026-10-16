import { describe, expect, it } from 'vitest'

import type { DesktopUpdateDownloadState } from '@/global'

import { $updatedNote, dismissUpdatedNote, noteUpdatedFrom, updateBannerState } from './update-notices'

const readyZip: DesktopUpdateDownloadState = { status: 'ready', version: '0.0.10', file: '/u/a.zip', kind: 'zip' }

describe('updateBannerState', () => {
  it('shows for a verified macOS zip', () => {
    expect(updateBannerState(readyZip)).toEqual({ version: '0.0.10', visible: true })
  })

  it('does not show for a dmg, a download in progress, or nothing', () => {
    expect(updateBannerState({ ...readyZip, kind: 'dmg' }).visible).toBe(false)
    expect(updateBannerState({ ...readyZip, status: 'downloading' }).visible).toBe(false)
    expect(updateBannerState(null).visible).toBe(false)
  })

  it('is hidden while onboarding, during a turn, after dismissal, or with auto-download off', () => {
    expect(updateBannerState(readyZip, { onboarding: true }).visible).toBe(false)
    expect(updateBannerState(readyZip, { turnActive: true }).visible).toBe(false)
    expect(updateBannerState(readyZip, { dismissed: true }).visible).toBe(false)
    expect(updateBannerState(readyZip, { autoDownload: false }).visible).toBe(false)
  })
})

describe('updated note', () => {
  it('appears once for the first launch after a swap, then is forgotten', () => {
    window.localStorage.removeItem('factr:updated-note-seen')
    $updatedNote.set(null)

    noteUpdatedFrom({ appVersion: '0.0.10', updatedFrom: '0.0.9' })
    expect($updatedNote.get()).toEqual({ from: '0.0.9', version: '0.0.10' })

    dismissUpdatedNote()
    expect($updatedNote.get()).toBeNull()

    noteUpdatedFrom({ appVersion: '0.0.10', updatedFrom: '0.0.9' })
    expect($updatedNote.get()).toBeNull()
  })

  it('ignores a missing or identical source version', () => {
    window.localStorage.removeItem('factr:updated-note-seen')
    noteUpdatedFrom({ appVersion: '0.0.10', updatedFrom: null })
    noteUpdatedFrom({ appVersion: '0.0.10', updatedFrom: '0.0.10' })
    noteUpdatedFrom(null)

    expect($updatedNote.get()).toBeNull()
  })
})
