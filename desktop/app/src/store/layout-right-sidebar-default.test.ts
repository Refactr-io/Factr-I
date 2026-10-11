import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// The right sidebar starts closed on a first launch; an install that already saved a side choice keeps it.

describe('right sidebar first-run default', () => {
  beforeEach(() => {
    window.localStorage.clear()
    vi.resetModules()
  })

  afterEach(() => {
    vi.resetModules()
  })

  it('closes the right sidebar when nothing was saved, and only once', async () => {
    const tree = await import('@/components/pane-shell/tree/store')
    const layout = await import('@/store/layout')

    layout.setFileBrowserOpen(true)
    expect(tree.$collapsedTreeSides.get().has('right')).toBe(false)

    expect(layout.applyRightSidebarFirstRunDefault(false, false)).toBe(true)
    expect(layout.$fileBrowserOpen.get()).toBe(false)
    expect(tree.$collapsedTreeSides.get().has('right')).toBe(true)

    // Later in the session the user opens it: a repeat of the one-shot must not close it again.
    layout.setFileBrowserOpen(true)
    layout.applyRightSidebarFirstRunDefault(false, true)
    expect(layout.$fileBrowserOpen.get()).toBe(true)
  })

  it('keeps a saved choice', async () => {
    const tree = await import('@/components/pane-shell/tree/store')
    const layout = await import('@/store/layout')

    layout.setFileBrowserOpen(true)
    layout.applyRightSidebarFirstRunDefault(true, false)
    expect(layout.$fileBrowserOpen.get()).toBe(true)
    expect(tree.$collapsedTreeSides.get().has('right')).toBe(false)
  })

  it('knows whether a side choice was saved before the app started', async () => {
    const fresh = await import('@/components/pane-shell/tree/store')

    expect(fresh.hadSavedSideState()).toBe(false)
    vi.resetModules()
    window.localStorage.setItem('factr.desktop.collapsedTreeSides.v1', '["right"]')

    const saved = await import('@/components/pane-shell/tree/store')

    expect(saved.hadSavedSideState()).toBe(true)
  })
})
