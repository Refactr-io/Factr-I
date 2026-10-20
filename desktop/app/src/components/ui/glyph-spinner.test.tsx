import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

import { GlyphSpinner } from './glyph-spinner'

afterEach(cleanup)

it('keeps the SDK glyph-spinner props while rendering the shared pixel loader without a JS ticker', () => {
  vi.useFakeTimers()
  const { container, rerender } = render(<GlyphSpinner spinner="braille" />)
  expect(screen.getByRole('status', { name: 'Loading' })).toBeTruthy()
  expect(container.querySelectorAll('rect')).toHaveLength(9)
  expect(vi.getTimerCount()).toBe(0)
  rerender(<GlyphSpinner ariaLabel="Reading files" paused spinner="orbit" />)
  expect(screen.getByRole('status', { name: 'Reading files' })).toBeTruthy()
  expect(container.querySelector('svg')?.getAttribute('data-paused')).toBe('true')
  vi.useRealTimers()
})
