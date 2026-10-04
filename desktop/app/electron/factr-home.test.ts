import path from 'node:path'

import { describe, expect, it } from 'vitest'

import { factrHome } from './factr-home'

describe('factrHome', () => {
  it('prefers FACTR_HOME and falls back to ~/.factr/engine', () => {
    expect(factrHome({ FACTR_HOME: '/tmp/j' }, '/Users/a')).toBe('/tmp/j')
    expect(factrHome({}, '/Users/a')).toBe(path.join('/Users/a', '.factr/engine'))
    expect(factrHome({ FACTR_HOME: '' }, '/Users/a')).toBe(path.join('/Users/a', '.factr/engine'))
  })
})
