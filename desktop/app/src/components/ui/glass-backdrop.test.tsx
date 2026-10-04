import { cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { GlassBackdrop } from './glass-backdrop'

vi.mock('liquid-glass-react', () => ({
  default: ({ children }: { children: React.ReactNode }) => <div data-testid="lg">{children}</div>
}))

function stubEnv(reduced: boolean) {
  vi.stubGlobal('matchMedia', (q: string) => ({
    addEventListener: () => {},
    matches: reduced && q.includes('reduced-motion'),
    removeEventListener: () => {}
  }))
  vi.stubGlobal(
    'ResizeObserver',
    class {
      disconnect() {}
      observe() {}
    }
  )
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 Chrome/140.0 Electron')
}

beforeEach(() => vi.restoreAllMocks())
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe('GlassBackdrop', () => {
  it('renders nothing under reduced motion', () => {
    stubEnv(true)
    const { container } = render(
      <div>
        <GlassBackdrop />
      </div>
    )
    expect(container.querySelector('[data-slot="glass-backdrop"]')).toBeNull()
    expect(container.firstElementChild?.hasAttribute('data-glass')).toBe(false)
  })

  it('renders the backdrop host and marks the parent otherwise', () => {
    stubEnv(false)
    const { container } = render(
      <div>
        <GlassBackdrop />
      </div>
    )
    expect(container.querySelector('[data-slot="glass-backdrop"]')).not.toBeNull()
    expect(container.firstElementChild?.hasAttribute('data-glass')).toBe(true)
  })
})
