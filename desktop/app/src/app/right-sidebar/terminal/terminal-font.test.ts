import { describe, expect, it, vi } from 'vitest'

import {
  applyTerminalFontFamily,
  DEFAULT_TERMINAL_FONT_FAMILY,
  prepareTerminalFontFamily,
  resolveTerminalFontFamily,
  terminalTypography,
  warmTerminalFontFamily
} from './terminal-font'

describe('terminal font resolution', () => {
  it('keeps the bundled stack when no preference is configured', () => {
    expect(resolveTerminalFontFamily('')).toBe(DEFAULT_TERMINAL_FONT_FAMILY)
    expect(resolveTerminalFontFamily(undefined)).toBe(DEFAULT_TERMINAL_FONT_FAMILY)
  })

  it('quotes a friendly family name and appends the bundled fallback stack', () => {
    expect(resolveTerminalFontFamily('  MesloLGS NF  ')).toBe(`'MesloLGS NF', ${DEFAULT_TERMINAL_FONT_FAMILY}`)
  })

  it('preserves an authored CSS stack before the bundled fallbacks', () => {
    expect(resolveTerminalFontFamily("'Hack Nerd Font', monospace")).toBe(
      `'Hack Nerd Font', monospace, ${DEFAULT_TERMINAL_FONT_FAMILY}`
    )
  })
})

describe('terminal font lifecycle', () => {
  it('restarts initial warming when config arrives late', async () => {
    let latest = 'fallback'

    const warm = vi.fn(async (fontFamily: string) => {
      if (fontFamily === 'fallback') {
        latest = 'MesloLGS NF'
      }
    })

    await expect(
      prepareTerminalFontFamily(
        () => latest,
        () => true,
        warm
      )
    ).resolves.toBe('MesloLGS NF')
    expect(warm.mock.calls.map(([font]) => font)).toEqual(['fallback', 'MesloLGS NF'])
  })

  it('cancels a stale initial font request before xterm mounts', async () => {
    let current = true

    const warm = vi.fn(async () => {
      current = false
    })

    await expect(
      prepareTerminalFontFamily(
        () => 'MesloLGS NF',
        () => current,
        warm
      )
    ).resolves.toBeNull()
  })

  it('does not paint a stale live font request', async () => {
    const term = {
      options: { fontFamily: 'newer' },
      rows: 24,
      refresh: vi.fn()
    }

    await expect(
      applyTerminalFontFamily({
        clearTextureAtlas: vi.fn(),
        fit: vi.fn(),
        fontFamily: 'stale',
        isCurrent: () => false,
        term,
        warm: vi.fn().mockResolvedValue(undefined)
      })
    ).resolves.toBe(false)

    expect(term.options.fontFamily).toBe('newer')
    expect(term.refresh).not.toHaveBeenCalled()
  })
})

describe('terminal typography', () => {
  it('reads calibrated CSS sizes for both terminal kinds and warms real bold/italic faces', async () => {
    const root = document.documentElement
    root.style.setProperty('--terminal-font-size', '13')
    root.style.setProperty('--terminal-line-height', '1.4')

    try {
      expect(terminalTypography()).toEqual({
        fontSize: 13,
        fontWeight: 400,
        fontWeightBold: 600,
        letterSpacing: 0,
        lineHeight: 1.4
      })

      const load = vi.fn().mockResolvedValue([])
      await warmTerminalFontFamily(DEFAULT_TERMINAL_FONT_FAMILY, { load })
      expect(load.mock.calls.map(([descriptor]) => descriptor)).toEqual(
        ['400', '600', 'italic 400', 'italic 600'].map(face => `${face} 13px ${DEFAULT_TERMINAL_FONT_FAMILY}`)
      )

      root.style.setProperty('--terminal-font-size', '15')
      expect(terminalTypography().fontSize).toBe(15)
    } finally {
      root.style.removeProperty('--terminal-font-size')
      root.style.removeProperty('--terminal-line-height')
    }
  })
})
