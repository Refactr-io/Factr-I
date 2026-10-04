/**
 * Fixed desktop palette model. White and black both have complete color sets.
 * Layout, sizing, radius, and line-height live in styles.css.
 */

export interface DesktopThemeColors {
  background: string
  foreground: string
  card: string
  cardForeground: string
  muted: string
  mutedForeground: string
  popover: string
  popoverForeground: string
  primary: string
  primaryForeground: string
  secondary: string
  secondaryForeground: string
  accent: string
  accentForeground: string
  border: string
  input: string
  /** Generic focus ring — buttons, inputs, etc. */
  ring: string
  /**
   * Brand-accent stroke — focus rings, streaming cursors, active session
   * pills, branded scrollbars, text selection. Falls back to `ring`.
   * Aliased to the DS `--midground` token.
   */
  midground?: string
  /** Auto-derived from `midground` luminance when omitted. */
  midgroundForeground?: string
  /** Composer outline / focus color. Falls back to `midground`. */
  composerRing?: string
  destructive: string
  destructiveForeground: string
  sidebarBackground?: string
  sidebarBorder?: string
  userBubble?: string
  userBubbleBorder?: string
}

export interface DesktopThemeTypography {
  fontSans: string
  fontMono: string
}

/**
 * Integrated-terminal ANSI palette (xterm `ITheme`, minus `background`).
 *
 * Populated only when a converted VS Code theme ships a full `terminal.ansi*`
 * set; otherwise the terminal keeps its built-in VS Code default palette.
 * `background` is intentionally absent — the pane always paints the live skin
 * surface so it stays translucent.
 */
export interface DesktopTerminalPalette {
  foreground?: string
  cursor?: string
  /** Keeps its source alpha — xterm blends it over the surface. */
  selectionBackground?: string
  black?: string
  red?: string
  green?: string
  yellow?: string
  blue?: string
  magenta?: string
  cyan?: string
  white?: string
  brightBlack?: string
  brightRed?: string
  brightGreen?: string
  brightYellow?: string
  brightBlue?: string
  brightMagenta?: string
  brightCyan?: string
  brightWhite?: string
}

export interface DesktopTheme {
  name: string
  label: string
  description: string
  /** White appearance. */
  colors: DesktopThemeColors
  /** Black appearance. */
  darkColors: DesktopThemeColors
  typography?: Partial<DesktopThemeTypography>
  /** Light-variant terminal ANSI palette (also the fallback for dark). */
  terminal?: DesktopTerminalPalette
  /** Dark-variant terminal ANSI palette. Falls back to `terminal`. */
  darkTerminal?: DesktopTerminalPalette
}
