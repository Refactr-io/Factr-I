import type { DesktopTheme, DesktopThemeTypography } from './types'

// Match the pre-theme CSS defaults; platform and emoji faces fill missing scripts.
export const EMOJI_FALLBACK = '"Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji", emoji'
export const DEFAULT_TYPOGRAPHY: DesktopThemeTypography = {
  fontSans:
    '"Pretendard", "Segoe WPC", "Segoe UI", -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", system-ui, sans-serif, ' +
    EMOJI_FALLBACK,
  fontMono: '"IBM Plex Mono", "Cascadia Code", "SF Mono", Menlo, Consolas, monospace, ' + EMOJI_FALLBACK
}

/** One fixed palette with two appearances, taken from the reference palette: a neutral canvas with a slightly darker
 * frame and sidebar, a lighter elevated card for the composer and inputs, slate ink text (light) and soft grey text
 * (dark), and hairline dividers. Light text levels are the ink at descending alpha; dark text levels are greys. Primary
 * actions are neutral (near-black in light, near-white in dark). The accent is blue, used for the Settings area and
 * the on-state of controls. */
export const factrTheme: DesktopTheme = {
  name: 'factr',
  label: 'Factr-I',
  description: 'White and black',
  typography: DEFAULT_TYPOGRAPHY,
  colors: {
    background: '#f8f8f8',
    foreground: '#132732',
    card: '#fafafa',
    cardForeground: '#132732',
    muted: '#f0f0f0',
    mutedForeground: '#4d5c64',
    popover: '#fdfdfd',
    popoverForeground: '#132732',
    primary: '#252525',
    primaryForeground: '#fbfbfb',
    secondary: '#f0f0f0',
    secondaryForeground: '#132732',
    accent: '#dbdbda',
    accentForeground: '#132732',
    border: '#e0e0e0',
    input: '#fafafa',
    ring: '#6e7a80',
    midground: '#6e7a80',
    midgroundForeground: '#fbfbfb',
    composerRing: '#6e7a80',
    destructive: '#c8372f',
    destructiveForeground: '#ffffff',
    sidebarBackground: '#e9e9e8',
    sidebarBorder: '#e0e0e0',
    userBubble: '#efefef',
    userBubbleBorder: '#e0e0e0'
  },
  darkColors: {
    background: '#161616',
    foreground: '#dedede',
    card: '#222222',
    cardForeground: '#dedede',
    muted: '#2a2a2a',
    mutedForeground: '#989898',
    popover: '#2a2a2a',
    popoverForeground: '#dedede',
    primary: '#dedede',
    primaryForeground: '#161616',
    secondary: '#2a2a2a',
    secondaryForeground: '#dedede',
    accent: '#2a2a2a',
    accentForeground: '#dedede',
    border: '#2d2d2d',
    input: '#222222',
    ring: '#9a9a9a',
    midground: '#9a9a9a',
    midgroundForeground: '#161616',
    composerRing: '#9a9a9a',
    destructive: '#eb534e',
    destructiveForeground: '#161616',
    sidebarBackground: '#1f1f1e',
    sidebarBorder: '#2d2d2d',
    userBubble: '#2a2a2a',
    userBubbleBorder: '#2d2d2d'
  }
}
