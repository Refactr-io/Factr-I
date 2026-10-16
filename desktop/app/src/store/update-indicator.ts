/**
 * The persistent "update available" mark on the rail. Unlike the one-time toast it has no snooze: it shows for as
 * long as the last successful check found a newer release, and nothing else (a failed check, an unsupported
 * install, being offline or onboarding) hides it. It reads the status the poller already keeps, so it adds no
 * network calls.
 */

import { atom, computed } from 'nanostores'

import type { DesktopUpdateDownloadState, DesktopUpdateStatus } from '@/global'
import { $onboardingSurfaces } from '@/store/onboarding-presence'
import { $autoDownload, $updateDownload } from '@/store/update-download'
import { $updateStatus } from '@/store/updates'

export type UpdateIndicatorPhase = 'available' | 'downloading' | 'ready'

export interface UpdateIndicatorState {
  /** available: a newer release exists; downloading / ready: its installer is being fetched / verified on disk. */
  phase: UpdateIndicatorPhase
  visible: boolean
  /** The newest release version when the feed names one (no leading "v"), else null. */
  version: null | string
}

const HIDDEN: UpdateIndicatorState = { phase: 'available', visible: false, version: null }

/** Visibility rules, pure so they can be tested without stores. */
export function updateIndicatorState(
  status: DesktopUpdateStatus | null | undefined,
  {
    autoDownload = true,
    download,
    onboarding = false,
    online = true
  }: { autoDownload?: boolean; download?: DesktopUpdateDownloadState | null; onboarding?: boolean; online?: boolean } = {}
): UpdateIndicatorState {
  if (!status || status.supported === false || status.error || onboarding || !online) {
    return HIDDEN
  }

  const behind = typeof status.behind === 'number' ? status.behind : 0
  const available = status.updateAvailable === true || behind > 0

  if (!available || !(status.targetSha || status.latestVersion)) {
    return HIDDEN
  }

  const version = status.latestVersion?.trim().replace(/^v/i, '') || null

  // The download state only counts for the version the feed currently offers, and only while auto-download is on.
  const forThisVersion = autoDownload && version !== null && download?.version === version

  const phase: UpdateIndicatorPhase =
    forThisVersion && download?.status === 'ready'
      ? 'ready'
      : forThisVersion && download?.status === 'downloading'
        ? 'downloading'
        : 'available'

  return { phase, visible: true, version }
}

/** Reactive online flag; the browser events are the only signal there is. */
export const $online = atom(typeof navigator === 'undefined' ? true : navigator.onLine !== false)

if (typeof window !== 'undefined') {
  window.addEventListener('online', () => $online.set(true))
  window.addEventListener('offline', () => $online.set(false))
}

export const $updateIndicator = computed(
  [$updateStatus, $onboardingSurfaces, $online, $updateDownload, $autoDownload],
  (status, surfaces, online, download, autoDownload) =>
    updateIndicatorState(status, { autoDownload, download, onboarding: surfaces.size > 0, online })
)
