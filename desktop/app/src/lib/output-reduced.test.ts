import { describe, expect, it } from 'vitest'

import { isOutputReduced, OUTPUT_REDUCED_MARKER } from './output-reduced'

describe('isOutputReduced', () => {
  it('sees the marker at the end of a string or an output field', () => {
    expect(isOutputReduced(`ok\n\n${OUTPUT_REDUCED_MARKER}`)).toBe(true)
    expect(isOutputReduced(`ok\n\n${OUTPUT_REDUCED_MARKER}\n`)).toBe(true)
    expect(isOutputReduced({ output: `x ${OUTPUT_REDUCED_MARKER}` })).toBe(true)
  })

  it('ignores results that merely mention it, and anything that is not text', () => {
    expect(isOutputReduced(`${OUTPUT_REDUCED_MARKER} is what rtk adds`)).toBe(false)
    expect(isOutputReduced('plain output')).toBe(false)
    expect(isOutputReduced(undefined)).toBe(false)
    expect(isOutputReduced(null)).toBe(false)
    expect(isOutputReduced(42)).toBe(false)
  })
})
