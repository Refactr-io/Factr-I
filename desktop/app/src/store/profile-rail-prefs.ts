import { Codecs, persistentAtom } from '@/lib/persisted'
import { modeBound } from '@/store/interface-mode'

// The colored profile strip at the sidebar foot. For someone who runs profiles
// as bots it duplicates the footer's gateway selector, so it can be switched
// off; while it is off the statusbar grows a profile dropdown beside the
// gateway switcher so switching profiles never loses its door. Off by default.
// Simple mode rests it hidden (unless it is the only door left) without
// touching this preference.
const $profileRailVisiblePref = persistentAtom('factr.desktop.profileRailVisible', false, Codecs.bool)

export const $profileRailVisible = modeBound('profileRailVisible', $profileRailVisiblePref, value =>
  $profileRailVisiblePref.set(value)
)

export function toggleProfileRailVisible() {
  $profileRailVisible.set(!$profileRailVisible.get())
}
