import type { DesktopTheme, DesktopThemeTypography } from './types'

// Match the pre-theme CSS defaults; platform and emoji faces fill missing scripts.
export const EMOJI_FALLBACK = '"Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji", emoji'
export const DEFAULT_TYPOGRAPHY: DesktopThemeTypography = {
  fontSans:
    '"Pretendard", "Segoe WPC", "Segoe UI", -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", system-ui, sans-serif, ' +
    EMOJI_FALLBACK,
  fontMono: '"IBM Plex Mono", "Cascadia Code", "SF Mono", Menlo, Consolas, monospace, ' + EMOJI_FALLBACK
}

/** One fixed palette with two appearances. Light values are the Perplexity DESIGN.md from styles.refero.design:
 * parchment canvas, soft-paper cards, ink text, graphite secondary text, warm-mist hairlines. The window frame
 * (title row and icon rail) is parchment pulled 40% toward warm mist: a neutral gray one step off the canvas,
 * like Codex. No blue or teal accent. Dark reuses its ink, charcoal and paper neutrals because the reference
 * has no dark spec. */
export const factrTheme: DesktopTheme = {
  name: 'factr',
  label: 'Factr-I',
  description: 'White and black',
  typography: DEFAULT_TYPOGRAPHY,
  colors: {
    background: '#faf8f5',
    foreground: '#27251e',
    card: '#fdfbfa',
    cardForeground: '#27251e',
    muted: '#f2f0ec',
    mutedForeground: '#625f5a',
    popover: '#fdfbfa',
    popoverForeground: '#27251e',
    primary: '#27251e',
    primaryForeground: '#faf8f5',
    secondary: '#f2f0ec',
    secondaryForeground: '#27251e',
    accent: '#ebe9e5',
    accentForeground: '#27251e',
    border: '#d1d1cd',
    input: '#fdfbfa',
    ring: '#6b6964',
    midground: '#6b6964',
    midgroundForeground: '#faf8f5',
    composerRing: '#6b6964',
    destructive: '#b33b35',
    destructiveForeground: '#fffdfc',
    sidebarBackground: '#eae8e5',
    sidebarBorder: '#d1d1cd',
    userBubble: '#f2f0ec',
    userBubbleBorder: '#d1d1cd'
  },
  darkColors: {
    background: '#161616',
    foreground: '#fffdfc',
    card: '#1e1e1e',
    cardForeground: '#fffdfc',
    muted: '#2a2a2a',
    mutedForeground: '#b6b6b6',
    popover: '#262626',
    popoverForeground: '#fffdfc',
    primary: '#fffdfc',
    primaryForeground: '#0a0a0a',
    secondary: '#2a2a2a',
    secondaryForeground: '#fffdfc',
    accent: '#2a2a2a',
    accentForeground: '#fffdfc',
    border: '#545454',
    input: '#1e1e1e',
    ring: '#d5d9e2',
    midground: '#d5d9e2',
    midgroundForeground: '#0a0a0a',
    composerRing: '#d5d9e2',
    destructive: '#ff5f57',
    destructiveForeground: '#0a0a0a',
    sidebarBackground: '#151515',
    sidebarBorder: '#545454',
    userBubble: '#2c2c2c',
    userBubbleBorder: '#545454'
  }
}
