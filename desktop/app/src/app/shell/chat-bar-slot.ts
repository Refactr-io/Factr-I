import { atom } from 'nanostores'

/** The row under the title bar, at the top of the primary chat surface, that hosts the pane icons. Null while no chat is mounted. */
export const $chatBarSlot = atom<HTMLElement | null>(null)

export const setChatBarSlot = (el: HTMLElement | null) => $chatBarSlot.set(el)

/** Where the chat title (with its session menu) mounts in the title bar. */
export const $titleBarSlot = atom<HTMLElement | null>(null)

export const setTitleBarSlot = (el: HTMLElement | null) => $titleBarSlot.set(el)
