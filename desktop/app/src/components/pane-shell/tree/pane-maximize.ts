import { atom } from 'nanostores'

/**
 * Fill: one pane covers the whole workspace and the chat steps aside; the same control restores everything.
 * The tree is never rewritten and nothing is unmounted: the renderer just treats every other pane (except the
 * sessions list) as gone while a pane is filled, so a restore returns exactly what the user had open.
 */
export const $maximizedPane = atom<null | string>(null)

export const restoreMaximizedPane = (): void => $maximizedPane.set(null)

export const maximizePane = (paneId: string): void => $maximizedPane.set(paneId)

export const toggleMaximizedPane = (paneId: string): void =>
  $maximizedPane.set($maximizedPane.get() === paneId ? null : paneId)
