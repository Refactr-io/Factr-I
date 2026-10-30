import { describe, expect, it } from 'vitest'

import { type ClearanceRect, TOAST_BASE_OFFSET_PX, toastBottomOffset } from './toast-clearance'

const rect = (left: number, top: number, right: number, bottom: number): ClearanceRect => ({
  bottom,
  height: bottom - top,
  left,
  right,
  top,
  width: right - left
})

describe('toastBottomOffset', () => {
  it('stays in the corner when there is no composer', () => {
    expect(toastBottomOffset([], 800, 600, 984)).toBe(TOAST_BASE_OFFSET_PX)
  })

  it('lifts the stack above a composer that shares its horizontal space', () => {
    // a 90 px tall composer docked to the bottom of an 800 px window, spanning the toast's x-range
    expect(toastBottomOffset([rect(300, 710, 1000, 800)], 800, 600, 984)).toBe(102)
  })

  it('ignores a composer that is somewhere else horizontally', () => {
    expect(toastBottomOffset([rect(20, 710, 400, 800)], 800, 600, 984)).toBe(TOAST_BASE_OFFSET_PX)
  })

  it('ignores a hidden or off-screen composer (a background tab)', () => {
    expect(toastBottomOffset([rect(300, 0, 300, 0)], 800, 600, 984)).toBe(TOAST_BASE_OFFSET_PX)
    expect(toastBottomOffset([rect(300, 900, 1000, 990)], 800, 600, 984)).toBe(TOAST_BASE_OFFSET_PX)
  })

  it('clears the tallest composer when several are visible', () => {
    const tall = rect(300, 600, 1000, 800)
    const short = rect(300, 740, 1000, 800)

    expect(toastBottomOffset([short, tall], 800, 600, 984)).toBe(212)
  })
})
