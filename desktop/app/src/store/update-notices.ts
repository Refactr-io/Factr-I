/**
 * Two small, dismissible notices tied to updating: the once-per-launch "Restart to update" banner (a verified macOS
 * update is on disk) and the one-time "Updated to X" note after a restart that swapped the app.
 */

import { atom, computed } from 'nanostores'

import type { DesktopUpdateDownloadState } from '@/global'
import { persistString, storedString } from '@/lib/storage'
import { $onboardingSurfaces } from '@/store/onboarding-presence'
import { $autoDownload, $updateDownload } from '@/store/update-download'
import { $turnActive } from '@/store/update-indicator'
import { $desktopVersion } from '@/store/updates'

export interface UpdateBannerState {
  version: null | string
  visible: boolean
}

/** Visibility rules for the banner, pure for tests. Only a verified macOS zip can restart in place. */
export function updateBannerState(
  download: DesktopUpdateDownloadState | null | undefined,
  { autoDownload = true, dismissed = false, onboarding = false, turnActive = false } = {}
): UpdateBannerState {
  const ready = autoDownload && download?.status === 'ready' && download.kind === 'zip' && Boolean(download.version)

  if (!ready || dismissed || onboarding || turnActive) {
    return { version: null, visible: false }
  }

  return { version: download?.version ?? null, visible: true }
}

/** Dismissed for this launch only: a still-ready update shows the banner again next launch. */
export const $updateBannerDismissed = atom(false)

export const $updateBanner = computed(
  [$updateDownload, $autoDownload, $updateBannerDismissed, $onboardingSurfaces, $turnActive],
  (download, autoDownload, dismissed, surfaces, turnActive) =>
    updateBannerState(download, { autoDownload, dismissed, onboarding: surfaces.size > 0, turnActive })
)

const UPDATED_NOTE_SEEN_KEY = 'factr:updated-note-seen'

/** "Updated to X" after a swap: set from the version info the main process reports on the first launch after one. */
export const $updatedNote = atom<null | { from: string; version: string }>(null)

export function noteUpdatedFrom(info: { appVersion?: string; updatedFrom?: null | string } | null | undefined): void {
  const from = info?.updatedFrom?.trim()
  const version = info?.appVersion?.trim()

  if (!from || !version || from === version || storedString(UPDATED_NOTE_SEEN_KEY) === version) {
    return
  }

  $updatedNote.set({ from, version })
}

export function dismissUpdatedNote(): void {
  const note = $updatedNote.get()

  if (note) {
    persistString(UPDATED_NOTE_SEEN_KEY, note.version)
  }

  $updatedNote.set(null)
}

$desktopVersion.subscribe(info => noteUpdatedFrom(info))
