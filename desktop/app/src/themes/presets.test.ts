import { contrastRatio } from '@factr/shared/color'
import { describe, expect, it } from 'vitest'

import { DEFAULT_TYPOGRAPHY, factrTheme } from './presets'

it('provides only one fixed palette with readable white and black appearances', () => {
  expect(factrTheme.name).toBe('factr')

  for (const colors of [factrTheme.colors, factrTheme.darkColors]) {
    expect(contrastRatio(colors.foreground, colors.background)).toBeGreaterThanOrEqual(4.5)
    expect(contrastRatio(colors.primaryForeground, colors.primary)).toBeGreaterThanOrEqual(4.5)
    expect(contrastRatio(colors.mutedForeground, colors.muted)).toBeGreaterThanOrEqual(4.5)
  }
})

// #40364: none of the UI text/mono fonts carry emoji glyphs, so every font
// stack must end with a color-emoji fallback or emoji render as tofu on
// platforms whose default font lacks them (e.g. Linux).
describe('theme typography emoji fallback (#40364)', () => {
  const stacks: Array<[string, string]> = [
    ['DEFAULT_TYPOGRAPHY.fontSans', DEFAULT_TYPOGRAPHY.fontSans],
    ['DEFAULT_TYPOGRAPHY.fontMono', DEFAULT_TYPOGRAPHY.fontMono],
    // A theme may override only fontMono (fontSans then falls back to the
    // default, which already carries the emoji stack), so skip undefined.
    ['factr.fontSans', factrTheme.typography?.fontSans ?? DEFAULT_TYPOGRAPHY.fontSans],
    ['factr.fontMono', factrTheme.typography?.fontMono ?? DEFAULT_TYPOGRAPHY.fontMono]
  ]

  it.each(stacks)('%s includes a color-emoji font', (_label, stack) => {
    expect(stack).toMatch(/Apple Color Emoji|Segoe UI Emoji|Noto Color Emoji|(^|,\s*)emoji\b/)
  })
})
