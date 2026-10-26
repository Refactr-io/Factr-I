import type { ITheme, Terminal } from '@xterm/xterm'
import type { CSSProperties } from 'react'

import { isMacPlatform } from '@/lib/platform'
import type { DesktopTerminalPalette } from '@/themes/types'

// The ANSI slots are VS Code's default integrated-terminal palette (terminalColorRegistry.ts) — a
// fixed table per theme type, not luminance-derived. Light/dark diverge on
// purpose so each stays legible (e.g. mustard yellow on white). Background, foreground, cursor and
// selection follow the app theme.
const DARK_THEME: ITheme = {
  background: '#151313',
  foreground: '#fffdfc',
  cursor: '#67eaa2',
  cursorAccent: '#151313',
  selectionBackground: '#67eaa247',
  black: '#151313',
  red: '#d9776d',
  green: '#84c79a',
  yellow: '#d6b25c',
  blue: '#86aee8',
  magenta: '#c4a3c0',
  cyan: '#b3a898',
  white: '#afaeac',
  brightBlack: '#818581',
  brightRed: '#e8948b',
  brightGreen: '#a3d9b4',
  brightYellow: '#e6c97f',
  brightBlue: '#a6c4f0',
  brightMagenta: '#d6bdd3',
  brightCyan: '#cbc2b4',
  brightWhite: '#fffdfc'
}

const LIGHT_THEME: ITheme = {
  background: '#fffdfc',
  foreground: '#151313',
  cursor: '#1d3023',
  cursorAccent: '#fffdfc',
  selectionBackground: '#67eaa261',
  black: '#151313',
  red: '#a8402f',
  green: '#356e46',
  yellow: '#8a6a1f',
  blue: '#2b5fa6',
  magenta: '#85607f',
  cyan: '#6f655a',
  white: '#575554',
  brightBlack: '#6e6c6c',
  brightRed: '#b4493c',
  brightGreen: '#3e7a4f',
  brightYellow: '#9a7425',
  brightBlue: '#3a6fb8',
  brightMagenta: '#95708f',
  brightCyan: '#7f7468',
  brightWhite: '#8a8786'
}

// Palette by painted mode, optionally overlaid with an imported theme's ANSI
// palette (Solarized terminal for the Solarized skin, etc.). `palette` only
// fills the slots it defines, so a partial import keeps the mode defaults for
// the rest. `background` is a fallback only — withSurface swaps in the live skin
// surface at runtime (keeping transparency); minimumContrastRatio keeps colors
// crisp against it.
export function terminalTheme(mode: 'light' | 'dark', palette?: DesktopTerminalPalette): ITheme {
  const base = mode === 'dark' ? DARK_THEME : LIGHT_THEME

  if (!palette) {
    return base
  }

  const overlay = { ...base } as Record<string, string>

  for (const [slot, value] of Object.entries(palette)) {
    if (value) {
      overlay[slot] = value
    }
  }

  return overlay as ITheme
}

// Resolve --ui-terminal-surface-background (a color-mix on the skin seed) to a
// concrete color for the WebGL renderer + contrast clamp. Custom props don't
// resolve via getComputedStyle, so probe a real background-color. Read AFTER
// applyTheme repaints (mount / rAF post-change) or it lags a frame behind.
export function resolveSurfaceColor(fallback: string): string {
  if (typeof document === 'undefined' || !document.body) {
    return fallback
  }

  const probe = document.createElement('span')
  probe.style.cssText =
    'position:absolute;visibility:hidden;pointer-events:none;background-color:var(--ui-terminal-surface-background)'
  document.body.appendChild(probe)
  const resolved = getComputedStyle(probe).backgroundColor
  probe.remove()

  return resolved && resolved !== 'rgba(0, 0, 0, 0)' ? resolved : fallback
}

export { isMacPlatform }

export function terminalSelectionLabel(term: Terminal, shellName: string, text: string) {
  const pos = term.getSelectionPosition()

  if (pos) {
    return pos.start.y === pos.end.y ? `${shellName}:${pos.start.y}` : `${shellName}:${pos.start.y}-${pos.end.y}`
  }

  const lines = Math.max(1, text.trim().split(/\r?\n/).length)

  return `${shellName}:${lines} line${lines === 1 ? '' : 's'}`
}

export function terminalSelectionAnchor(host: HTMLDivElement): CSSProperties | null {
  const rect = Array.from(host.querySelectorAll<HTMLElement>('.xterm-selection div'))
    .map(node => node.getBoundingClientRect())
    .filter(r => r.width > 0 && r.height > 0)
    .at(-1)

  if (!rect) {
    return null
  }

  const hostRect = host.getBoundingClientRect()
  const buttonWidth = 128
  const left = Math.min(Math.max(rect.left - hostRect.left, 8), Math.max(8, host.clientWidth - buttonWidth - 8))
  const top = Math.min(Math.max(rect.bottom - hostRect.top + 4, 8), Math.max(8, host.clientHeight - 34))

  return { left, top }
}
