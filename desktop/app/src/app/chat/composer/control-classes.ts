import { cn } from '@/lib/utils'

// Shared class names for the composer's control row, in a module of their own
// so both the row (`controls.tsx`) and the menus it renders can wear them
// without importing each other in a cycle.

export const ICON_BTN = 'size-(--composer-control-size) shrink-0 rounded-md'

export const GHOST_ICON_BTN = cn(
  ICON_BTN,
  'text-(--ui-text-tertiary) hover:bg-(--chrome-action-hover) hover:text-foreground'
)

// Send/voice-conversation primary: solid foreground-on-background (reads as
// black-on-white in light mode, white-on-black in dark mode), the row's one
// high-contrast CTA. A squircle, not a circle: it sits ~8px inside the
// composer's 10px superellipse corner, so its corner follows the box's.
export const PRIMARY_ICON_BTN = cn(
  'size-(--composer-control-primary-size,var(--composer-control-size)) shrink-0 rounded-[7px] p-0',
  'bg-foreground text-background hover:bg-foreground/90',
  'disabled:bg-foreground/30 disabled:text-background disabled:opacity-100'
)

/** The mic while it records or transcribes: Claude's blue box, white glyph. */
export const DICTATING_ICON_BTN = 'bg-[#2f6fdb] text-white hover:bg-[#2a63c4] hover:text-white'

/** A toggle that is currently ON — dictation, spoken replies, the wake word. */
export const ACTIVE_ICON_BTN = 'bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary'

// Claude Code's send: a bare return glyph inside the input box, not a filled
// circle. Quiet until there is something to send, then full ink.
export const RETURN_ICON_BTN = cn(
  ICON_BTN,
  'p-0 text-(--ui-text-secondary) hover:bg-(--chrome-action-hover) hover:text-foreground',
  'disabled:text-(--ui-text-quaternary) disabled:opacity-100'
)
