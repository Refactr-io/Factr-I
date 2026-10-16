/**
 * The persistent "update available" mark on the rail. Unlike the one-time toast it has no snooze: it shows for as
 * long as the last successful check found a newer release, and nothing else (a failed check, an unsupported
 * install, being offline or onboarding) hides it. It reads the status the poller already keeps, so it adds no
 * network calls.
 */

import { atom, computed } from 'nanostores'

import type { DesktopUpdateStatus } from '@/global'
import { $onboardingSurfaces } from '@/store/onboarding-presence'
import { $updateStatus } from '@/store/updates'

export interface UpdateIndicatorState {
  visible: boolean
  /** The newest release version when the feed names one (no leading "v"), else null. */
  version: null | string
}

const HIDDEN: UpdateIndicatorState = { visible: false, version: null }

/** Visibility rules, pure so they can be tested without stores. */
export function updateIndicatorState(
  status: DesktopUpdateStatus | null | undefined,
  { onboarding = false, online = true }: { onboarding?: boolean; online?: boolean } = {}
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

  return { visible: true, version }
}

/** Reactive online flag; the browser events are the only signal there is. */
export const $online = atom(typeof navigator === 'undefined' ? true : navigator.onLine !== false)

if (typeof window !== 'undefined') {
  window.addEventListener('online', () => $online.set(true))
  window.addEventListener('offline', () => $online.set(false))
}

export const $updateIndicator = computed([$updateStatus, $onboardingSurfaces, $online], (status, surfaces, online) =>
  updateIndicatorState(status, { onboarding: surfaces.size > 0, online })
)
