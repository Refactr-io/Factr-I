import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { modePref, ThemeProvider, useTheme } from './context'

const cssVar = (name: string) => document.documentElement.style.getPropertyValue(name)

describe('fixed white and black desktop appearances', () => {
  let theme: ReturnType<typeof useTheme>

  function Probe() {
    theme = useTheme()

    return null
  }

  beforeEach(() => {
    localStorage.clear()
    document.documentElement.classList.remove('dark')
  })
  afterEach(cleanup)

  it('paints only the fixed palette and ignores a saved legacy skin', () => {
    localStorage.setItem('factr-desktop-theme-v2', 'catppuccin')
    modePref.assign('default', 'light')
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    )
    expect(theme.themeName).toBe('factr')
    expect(cssVar('--theme-background-seed')).toBe('#faf8f5')
    expect(cssVar('--theme-primary')).toBe('#27251e')
  })

  it('switches to black, persists it, and rejects a third mode', () => {
    modePref.assign('default', 'light')
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    )
    act(() => theme.setMode('dark'))
    expect(theme.mode).toBe('dark')
    expect(modePref.resolve('default')).toBe('dark')
    expect(cssVar('--theme-background-seed')).toBe('#0a0a0a')
    expect(cssVar('--theme-primary')).toBe('#ffffff')
    act(() => theme.setMode('system' as 'dark'))
    expect(theme.mode).toBe('dark')
    expect(modePref.resolve('default')).toBe('dark')
  })

  it('prefers the current default mode over an old profile record', () => {
    localStorage.setItem('factr-desktop-profile-modes-v1', JSON.stringify({ default: 'system' }))
    modePref.assign('default', 'dark')
    expect(modePref.resolve('default')).toBe('dark')
  })
})
