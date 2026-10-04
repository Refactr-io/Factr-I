import { atom } from 'nanostores'

import { persistString, storedString } from '@/lib/storage'

/** What Enter does with a message typed while a reply is running: `queue` it
 *  for when the turn ends (Claude Code / Codex), or `steer` it into the live
 *  turn now. ⌘/Ctrl+Enter always does the other one. */
export type BusySendMode = 'queue' | 'steer'

const KEY = 'factr.desktop.composer.busy-send'

export const $busySendMode = atom<BusySendMode>(storedString(KEY) === 'steer' ? 'steer' : 'queue')

export function setBusySendMode(mode: BusySendMode) {
  $busySendMode.set(mode)
  persistString(KEY, mode)
}
