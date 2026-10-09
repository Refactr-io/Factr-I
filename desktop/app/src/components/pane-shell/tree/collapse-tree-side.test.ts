import { atom } from 'nanostores'
import { describe, expect, it } from 'vitest'

import { $collapsedTreeSides, bindTreeSideVisibility, collapseTreeSide } from './store'

describe('collapseTreeSide', () => {
  it('collapses the side and tells the store that owns its toggle, so the next click shows it', () => {
    const $open = atom(true)
    bindTreeSideVisibility('right', $open, open => $open.set(open))

    collapseTreeSide('right')

    expect($collapsedTreeSides.get().has('right')).toBe(true)
    expect($open.get()).toBe(false)

    // The toggle now reads "closed", so one press opens it again.
    $open.set(!$open.get())
    expect($open.get()).toBe(true)
    expect($collapsedTreeSides.get().has('right')).toBe(false)
  })
})
