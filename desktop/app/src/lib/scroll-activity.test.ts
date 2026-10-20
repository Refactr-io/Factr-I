import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { installScrollActivity } from './scroll-activity'

let uninstall: () => void

beforeEach(() => {
  vi.useFakeTimers()
  uninstall = installScrollActivity()
})

afterEach(() => {
  uninstall()
  vi.useRealTimers()
})

it('shows the bar while an element scrolls and clears it after idle plus the fade', () => {
  const el = document.createElement('div')

  document.body.append(el)
  el.dispatchEvent(new Event('scroll'))

  vi.advanceTimersByTime(400)
  expect(el.style.getPropertyValue('--sb-a')).toBe('1.000')

  vi.advanceTimersByTime(800 + 400)
  expect(el.style.getPropertyValue('--sb-a')).toBe('')
})
