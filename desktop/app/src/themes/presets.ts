import type { DesktopTheme, DesktopThemeTypography } from './types'

// Match the pre-theme CSS defaults; platform and emoji faces fill missing scripts.
export const EMOJI_FALLBACK = '"Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji", emoji'
export const DEFAULT_TYPOGRAPHY: DesktopThemeTypography = {
  fontSans:
    '"Pretendard", "Segoe WPC", "Segoe UI", -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", system-ui, sans-serif, ' +
    EMOJI_FALLBACK,
  fontMono: '"IBM Plex Mono", "Cascadia Code", "SF Mono", Menlo, Consolas, monospace, ' + EMOJI_FALLBACK
}

/** One fixed palette with two appearances, from the Refactr design system. Light: Rose Alloy canvas, Blackthorn Ink text,
 * a warm ink tint for the frame and sidebar, hairlines that are ink at low alpha. Dark: Blackthorn Ink canvas and the
 * dark neutral ladder (Charcoal for the frame and raised bands, Graphite Lift for lines, Iron for cards and menus),
 * Rose Alloy text. Spring Verdant is the focus ring on dark; Forest Black is the focus ring on light. No pure black or
 * pure white anywhere. Status colours are muted and warm (none between teal and cyan). */
export const factrTheme: DesktopTheme = {
  name: 'factr',
  label: 'Factr-I',
  description: 'Rose Alloy and Blackthorn Ink',
  typography: DEFAULT_TYPOGRAPHY,
  colors: {
    background: '#fffdfc',
    foreground: '#151313',
    card: '#fffdfc',
    cardForeground: '#151313',
    muted: '#f1efee',
    mutedForeground: '#575554',
    popover: '#fffdfc',
    popoverForeground: '#151313',
    primary: '#151313',
    primaryForeground: '#fffdfc',
    secondary: '#f1efee',
    secondaryForeground: '#151313',
    accent: '#e8e6e5',
    accentForeground: '#151313',
    border: '#e3e1e0',
    input: '#fffdfc',
    ring: '#1d3023',
    midground: '#6e6c6c',
    midgroundForeground: '#fffdfc',
    composerRing: '#6e6c6c',
    destructive: '#a8402f',
    destructiveForeground: '#fffdfc',
    sidebarBackground: '#f1efee',
    sidebarBorder: '#e3e1e0',
    userBubble: '#f1efee',
    userBubbleBorder: '#e3e1e0'
  },
  darkColors: {
    background: '#151313',
    foreground: '#fffdfc',
    card: '#1e1e1d',
    cardForeground: '#fffdfc',
    muted: '#2f2f2f',
    mutedForeground: '#afaeac',
    popover: '#353534',
    popoverForeground: '#fffdfc',
    primary: '#fffdfc',
    primaryForeground: '#151313',
    secondary: '#2f2f2f',
    secondaryForeground: '#fffdfc',
    accent: '#353534',
    accentForeground: '#fffdfc',
    border: '#2f2f2f',
    input: '#1e1e1d',
    ring: '#67eaa2',
    midground: '#afaeac',
    midgroundForeground: '#151313',
    composerRing: '#afaeac',
    destructive: '#e8948b',
    destructiveForeground: '#151313',
    sidebarBackground: '#1e1e1d',
    sidebarBorder: '#2f2f2f',
    userBubble: '#2f2f2f',
    userBubbleBorder: '#2f2f2f'
  }
}
