/**
 * Renderer side of the background update download: the main process downloads and verifies the installer
 * (electron/update-download.ts) and pushes its state here. Failures are silent by design: they only mean the
 * indicator keeps its plain "available" behaviour and the click opens the updates overlay.
 */

import { atom } from 'nanostores'

import type { DesktopUpdateDownloadState } from '@/global'
import { translateNow } from '@/i18n'
import { notify } from '@/store/notifications'

const IDLE: DesktopUpdateDownloadState = { status: 'idle', version: null, file: null, kind: null }

export const $updateDownload = atom<DesktopUpdateDownloadState>(IDLE)
/** The "Download updates automatically" setting (default ON, persisted by the main process). */
export const $autoDownload = atom<boolean>(true)

/** "Updated to X" after a swap: the main process reports { from, to } once, on the first launch after one. */
export const $updatedNote = atom<null | { from: string; version: string }>(null)

export function noteUpdated(record: { from?: string; to?: string } | null | undefined): void {
  const from = record?.from?.trim()
  const version = record?.to?.trim()

  if (from && version && from !== version) {
    $updatedNote.set({ from, version })
  }
}

export function dismissUpdatedNote(): void {
  $updatedNote.set(null)
}

let started = false

export function startUpdateDownloadSync(): void {
  const bridge = typeof window === 'undefined' ? undefined : window.factrDesktop?.updates

  if (started || !bridge) {
    return
  }

  started = true
  bridge.onDownloadState?.(state => $updateDownload.set(state))
  void bridge.getDownloadState?.().then(state => state && $updateDownload.set(state), () => undefined)
  void bridge.getPostUpdate?.().then(record => record && noteUpdated(record), () => undefined)
  void bridge.getAutoDownload?.().then(value => $autoDownload.set(value !== false), () => undefined)
}

export async function setAutoDownload(enabled: boolean): Promise<void> {
  $autoDownload.set(enabled)

  try {
    const saved = await window.factrDesktop?.updates.setAutoDownload?.(enabled)

    if (typeof saved === 'boolean') {
      $autoDownload.set(saved)
    }
  } catch {
    // Keep the optimistic value; the main process falls back to its default next launch.
  }
}

/** Auto-update is off: the indicator's click starts the download itself. Never throws. */
export async function startDownloadNow(): Promise<void> {
  try {
    await window.factrDesktop?.updates.downloadNow?.()
  } catch {
    // Silent: the indicator stays on "available" and its click can try again.
  }
}

/**
 * Install the verified update: on macOS the app swaps itself and restarts, elsewhere the installer is opened for the
 * user. When the automatic swap fails the main process reveals the download and we say so quietly (an info notice,
 * never an error). Never throws.
 */
export async function installDownloadedUpdate(): Promise<void> {
  try {
    const result = await window.factrDesktop?.updates.install?.()

    if (result?.fallback) {
      notify({
        kind: 'info',
        message: translateNow('notifications.updateInstallFallback'),
        title: translateNow('notifications.updateFactr')
      })
    }
  } catch {
    // Nothing to show: a failed open must not become an error dot.
  }
}

export function resetUpdateDownloadForTests(): void {
  started = false
  $updateDownload.set(IDLE)
  $autoDownload.set(true)
  $updatedNote.set(null)
}
