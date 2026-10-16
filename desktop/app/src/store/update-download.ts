/**
 * Renderer side of the background update download: the main process downloads and verifies the installer
 * (electron/update-download.ts) and pushes its state here. Failures are silent by design: they only mean the
 * indicator keeps its plain "available" behaviour and the click opens the updates overlay.
 */

import { atom } from 'nanostores'

import type { DesktopUpdateDownloadState } from '@/global'

const IDLE: DesktopUpdateDownloadState = { status: 'idle', version: null, file: null }

export const $updateDownload = atom<DesktopUpdateDownloadState>(IDLE)
/** The "Download updates automatically" setting (default ON, persisted by the main process). */
export const $autoDownload = atom<boolean>(true)

let started = false

export function startUpdateDownloadSync(): void {
  const bridge = typeof window === 'undefined' ? undefined : window.factrDesktop?.updates

  if (started || !bridge) {
    return
  }

  started = true
  bridge.onDownloadState?.(state => $updateDownload.set(state))
  void bridge.getDownloadState?.().then(state => state && $updateDownload.set(state), () => undefined)
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

/** Open the verified installer (or, failing that, the release page). Never throws. */
export async function installDownloadedUpdate(): Promise<void> {
  try {
    await window.factrDesktop?.updates.install?.()
  } catch {
    // Nothing to show: a failed open must not become an error dot.
  }
}

export function resetUpdateDownloadForTests(): void {
  started = false
  $updateDownload.set(IDLE)
  $autoDownload.set(true)
}
