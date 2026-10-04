/** Fixed Factr desktop palette: one white and one black appearance. */
import { useStore } from '@nanostores/react'
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useState } from 'react'

import { matchesQuery } from '@/hooks/use-media-query'
import { persistString, persistStringRecord, storedString, storedStringRecord } from '@/lib/storage'
import { $activeGatewayProfile, normalizeProfileKey } from '@/store/profile'
import { setAppearance } from '@/store/translucency'

import { $chatFontFamily, resolveChatFontFamily } from './chat-font'
import { readableInk } from './color'
import { DEFAULT_TYPOGRAPHY, factrTheme } from './presets'
import type { DesktopTheme } from './types'

export type ThemeMode = 'light' | 'dark'

const MODE_KEY = 'factr-desktop-mode-v1'
const PROFILE_MODES_KEY = 'factr-desktop-profile-modes-v1'
const LAST_PROFILE_KEY = 'factr-desktop-active-profile-v1'

const defaultMode = (): ThemeMode => (matchesQuery('(prefers-color-scheme: dark)') ? 'dark' : 'light')

const normalizeMode = (value: string | null): ThemeMode =>
  value === 'light' || value === 'dark' ? value : defaultMode()

// Old "system" and old profile skins resolve to a concrete light/dark mode.
// No stored skin value is read, so a legacy or imported palette cannot reactivate.
const profilePref = (record: string, legacy: string) => {
  const stored = (profile: string): string | null =>
    profile === 'default'
      ? (storedString(legacy) ?? storedStringRecord(record)[profile] ?? null)
      : (storedStringRecord(record)[profile] ?? storedString(legacy))

  return {
    stored,
    resolve: (profile: string): ThemeMode => normalizeMode(stored(profile)),
    assign: (profile: string, value: ThemeMode): void => {
      if (value !== 'light' && value !== 'dark') {
        return
      }

      if (profile === 'default') {
        persistString(legacy, value)
      } else {
        persistStringRecord(record, { ...storedStringRecord(record), [profile]: value })
      }
    }
  }
}

export const modePref = profilePref(PROFILE_MODES_KEY, MODE_KEY)
const readBootProfileKey = () => normalizeProfileKey(storedString(LAST_PROFILE_KEY))
const rememberActiveProfileKey = (profile: string) => persistString(LAST_PROFILE_KEY, profile)

const mixesFor = (): Record<string, string> => ({
  '--theme-mix-chrome': '100%',
  '--theme-mix-sidebar': '100%',
  '--theme-mix-card': '100%',
  '--theme-mix-elevated': '100%',
  '--theme-mix-bubble': '100%'
})

function applyTheme(mode: ThemeMode, chatFontFamily = $chatFontFamily.get()) {
  if (typeof document === 'undefined') {
    return
  }

  const root = document.documentElement
  const c = mode === 'dark' ? factrTheme.darkColors : factrTheme.colors
  const typo = DEFAULT_TYPOGRAPHY
  const rendered = mode
  const isDark = mode === 'dark'
  const midground = c.midground ?? c.ring

  root.style.setProperty('color-scheme', rendered)
  root.dataset.factrMode = rendered
  root.classList.toggle('dark', isDark)

  // Translucency follows the painted light/dark mode.
  setAppearance(rendered)

  // Brand seeds feed every glass + shadcn token via `color-mix()` in styles.css.
  const seeds: Record<string, string> = {
    '--theme-foreground': c.foreground,
    '--theme-primary': c.primary,
    '--theme-secondary': c.secondary,
    '--theme-accent-soft': c.accent,
    '--theme-midground': midground,
    '--theme-warm': c.primary,
    '--theme-background-seed': c.background,
    '--theme-sidebar-seed': c.sidebarBackground ?? c.background,
    '--theme-card-seed': c.card,
    '--theme-elevated-seed': c.popover,
    '--theme-bubble-seed': c.userBubble ?? c.popover
  }

  // shadcn/Tailwind tokens that aren't derived from the seed chain.
  const palette: Record<string, string> = {
    '--dt-primary-foreground': c.primaryForeground,
    '--dt-secondary-foreground': c.secondaryForeground,
    '--dt-accent-foreground': c.accentForeground,
    '--dt-border': c.border,
    '--dt-input': c.input,
    '--dt-ring': c.ring,
    '--dt-muted': c.muted,
    '--dt-midground-foreground': c.midgroundForeground ?? readableInk(midground),
    '--dt-primary-solid': c.primary,
    '--dt-primary-solid-foreground': c.primaryForeground,
    '--dt-composer-ring': c.composerRing ?? midground,
    '--dt-destructive': c.destructive,
    '--dt-destructive-foreground': c.destructiveForeground,
    '--dt-sidebar-border': c.sidebarBorder ?? c.border,
    '--dt-user-bubble-border': c.userBubbleBorder ?? c.border,
    // The reference palette's green is reserved for semantic success.
    '--ui-success': '#28c840',
    '--dt-font-sans': resolveChatFontFamily(chatFontFamily, typo.fontSans),
    '--dt-font-mono': typo.fontMono,
    '--noise-opacity-mul': isDark ? 'calc(0.04 / 0.21)' : 'calc(0.34 / 0.21)'
  }

  for (const [k, v] of Object.entries({ ...seeds, ...mixesFor(), ...palette })) {
    root.style.setProperty(k, v)
  }

  const chromeBg = c.background

  window.factrDesktop?.setTitleBarTheme?.({
    background: chromeBg,
    foreground: c.foreground
  })

  // The pre-paint script reads only the mode, then picks its fixed color.
  try {
    window.localStorage.setItem('factr-boot-color-scheme', rendered)
  } catch {
    // Storage may be unavailable (private mode / quota); the inline script
    // falls back to prefers-color-scheme.
  }
}

// A concrete mode also pins the native chrome. The OS preference is only the
// initial value when a profile has no saved mode.
const syncNativeTheme = (mode: ThemeMode) => window.factrDesktop?.setNativeTheme?.(mode)

if (typeof window !== 'undefined') {
  // Retire saved palette definitions and selections from earlier Desktop builds.
  // They have no activation path in the fixed two-mode renderer.
  try {
    for (const key of [
      'factr-desktop-theme-v2',
      'factr-desktop-profile-themes-v1',
      'factr-desktop-user-themes-v1',
      'factr-desktop-backend-themes-v1',
      'factr-boot-background'
    ]) {
      window.localStorage.removeItem(key)
    }
  } catch {
    // A restricted storage context still gets the fixed palette.
  }

  const bootProfile = readBootProfileKey()
  const mode = modePref.resolve(bootProfile)
  modePref.assign(bootProfile, mode)
  applyTheme(mode)
  syncNativeTheme(mode)
}

interface ThemeContextValue {
  theme: DesktopTheme
  themeName: string
  mode: ThemeMode
  resolvedMode: ThemeMode
  renderedMode: ThemeMode
  setMode: (mode: ThemeMode) => void
}

const ThemeContext = createContext<ThemeContextValue>({
  theme: factrTheme,
  themeName: factrTheme.name,
  mode: 'light',
  resolvedMode: 'light',
  renderedMode: 'light',
  setMode: () => {}
})

export function ThemeProvider({ children }: { children: ReactNode }) {
  const profileKey = normalizeProfileKey(useStore($activeGatewayProfile))

  const [mode, setModeState] = useState<ThemeMode>(() =>
    typeof window === 'undefined' ? 'light' : modePref.resolve(readBootProfileKey())
  )

  const chatFontFamily = useStore($chatFontFamily)

  useEffect(() => {
    rememberActiveProfileKey(profileKey)
    const next = modePref.resolve(profileKey)
    modePref.assign(profileKey, next)
    setModeState(next)
  }, [profileKey])

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key && event.key !== MODE_KEY && event.key !== PROFILE_MODES_KEY) {
        return
      }

      setModeState(modePref.resolve(normalizeProfileKey($activeGatewayProfile.get())))
    }

    window.addEventListener('storage', onStorage)

    return () => window.removeEventListener('storage', onStorage)
  }, [])

  useEffect(() => applyTheme(mode, chatFontFamily), [mode, chatFontFamily])
  useEffect(() => syncNativeTheme(mode), [mode])

  const setMode = useCallback((next: ThemeMode) => {
    if (next !== 'light' && next !== 'dark') {
      return
    }

    setModeState(next)
    modePref.assign(normalizeProfileKey($activeGatewayProfile.get()), next)
  }, [])

  const value = useMemo<ThemeContextValue>(
    () => ({
      theme: { ...factrTheme, colors: mode === 'dark' ? factrTheme.darkColors : factrTheme.colors },
      themeName: factrTheme.name,
      mode,
      resolvedMode: mode,
      renderedMode: mode,
      setMode
    }),
    [mode, setMode]
  )

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
}

export const useTheme = (): ThemeContextValue => useContext(ThemeContext)
