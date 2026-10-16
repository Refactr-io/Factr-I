/**
 * Two small, dismissible notices tied to updating: the once-per-launch "Restart to update" banner (a verified macOS
 * update is on disk) and the one-time "Updated to X" note after a restart that swapped the app.
 */

import { atom, computed } from 'nanostores'

import type { DesktopUpdateDownloadState } from '@/global'
import { $onboardingSurfaces } from '@/store/onboarding-presence'
import { $updateDownload } from '@/store/update-download'
import { $turnActive } from '@/store/update-indicator'

export interface UpdateBannerState {
  version: null | string
  visible: boolean
}

/** Visibility rules for the banner, pure for tests. Only a verified macOS zip can restart in place. */
export function updateBannerState(
  download: DesktopUpdateDownloadState | null | undefined,
  { dismissed = false, onboarding = false, turnActive = false } = {}
): UpdateBannerState {
  const ready = download?.status === 'ready' && download.kind === 'zip' && Boolean(download.version)

  if (!ready || dismissed || onboarding || turnActive) {
    return { version: null, visible: false }
  }

  return { version: download?.version ?? null, visible: true }
}

/** Dismissed for this launch only: a still-ready update shows the banner again next launch. */
export const $updateBannerDismissed = atom(false)

export const $updateBanner = computed(
  [$updateDownload, $updateBannerDismissed, $onboardingSurfaces, $turnActive],
  (download, dismissed, surfaces, turnActive) =>
    updateBannerState(download, { dismissed, onboarding: surfaces.size > 0, turnActive })
)

export { $updatedNote, dismissUpdatedNote, noteUpdated } from '@/store/update-download'
