import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { PaneVisibleContext } from '@/components/pane-shell/pane-visibility'
import { Loader2 } from '@/lib/icons'

import { Loader } from './loader'

beforeEach(() => {
  vi.useFakeTimers()
  vi.spyOn(globalThis.document, 'hasFocus').mockReturnValue(true)
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

describe('Loader', () => {
  it('delays the visible elapsed time until 3s and reserves its width throughout', () => {
    const { container } = render(<Loader label="Reading files" />)
    const timer = screen.getByText('0s')
    expect(timer.style.visibility).toBe('hidden')
    expect(timer.style.inlineSize).toBe('8ch')
    expect(screen.getByRole('status', { name: 'Reading files' })).toBeTruthy()
    act(() => vi.advanceTimersByTime(2999))
    expect(timer.style.visibility).toBe('hidden')
    act(() => vi.advanceTimersByTime(1))
    expect(timer.textContent).toBe('3s')
    expect(timer.style.visibility).toBe('visible')
    act(() => vi.advanceTimersByTime(57000))
    expect(timer.textContent).toBe('1:00')
    expect(timer.style.inlineSize).toBe('8ch')
    expect(container.querySelectorAll('rect')).toHaveLength(9)
  })

  it('uses supplied elapsed seconds without starting another clock', () => {
    const { rerender } = render(<Loader label="Running bash" seconds={2} />)
    expect(screen.getByText('2s').style.visibility).toBe('hidden')
    rerender(<Loader label="Running bash" seconds={3} />)
    expect(screen.getByText('3s').style.visibility).toBe('visible')
    expect(vi.getTimerCount()).toBe(0)
  })

  it.each(['Drive', 'Dots', 'Orbit'] as const)('renders the %s grid with decorative pixels', variant => {
    const { container } = render(<Loader ariaLabel="Working" variant={variant} />)
    expect(container.querySelectorAll('rect')).toHaveLength(9)
    expect(container.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true')
    expect(vi.getTimerCount()).toBe(0)
  })

  it('pauses the same mark in hidden panes and while fading out', () => {
    const { container, rerender } = render(
      <PaneVisibleContext.Provider value={false}>
        <Loader label="Reading" />
      </PaneVisibleContext.Provider>
    )

    expect(container.querySelector('svg')?.getAttribute('data-paused')).toBe('true')
    expect(vi.getTimerCount()).toBe(0)
    rerender(<Loader label="Reading" paused />)
    expect(container.querySelector('svg')?.getAttribute('data-paused')).toBe('true')
    rerender(<Loader label="Reading" />)
    expect(container.querySelector('svg')?.hasAttribute('data-paused')).toBe(false)
  })

  it('routes legacy ring icons to the grid and prevents caller spin classes rotating it', () => {
    const { container } = render(<Loader2 className="size-3 animate-spin" />)
    expect(container.querySelectorAll('rect')).toHaveLength(9)
    expect(container.querySelector('svg')?.style.animation).toBe('none')
  })
})
