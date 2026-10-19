import { atom } from 'nanostores'

/** The session rail: environment, context, connectors, outputs and changes for the focused chat. */
export const $sessionRailOpen = atom(false)

export const toggleSessionRail = () => $sessionRailOpen.set(!$sessionRailOpen.get())
