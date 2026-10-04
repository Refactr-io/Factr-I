/**
 * Window text size (zoom).
 *
 * The main process owns the zoom level and persists it (see electron/zoom.ts
 * for the scale). The renderer only mirrors the current percent for the
 * settings UI: preset clicks go to the main process over IPC, and every
 * change comes back through onChanged, including ones made with the
 * Ctrl/Cmd +/-/0 shortcuts or the View menu, so the UI never drifts.
 */

import { atom } from 'nanostores'

// Mirror DEFAULT_ZOOM_LEVEL (0 = 100%) so Appearance doesn't flash another value before
// the main-process zoom.get() resolves. Keep in sync with electron/zoom.ts.
export const $zoomPercent = atom<number>(100)

export function setZoomPercent(percent: number): void {
  window.factrDesktop?.zoom?.setPercent(percent)
}

if (typeof window !== 'undefined' && window.factrDesktop?.zoom) {
  void window.factrDesktop.zoom.get().then(({ percent }) => $zoomPercent.set(percent))
  window.factrDesktop.zoom.onChanged(({ percent }) => $zoomPercent.set(percent))
}
