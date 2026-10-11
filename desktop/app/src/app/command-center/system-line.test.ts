import { describe, expect, it } from 'vitest'

import { systemStatusLine } from './system-line'

const fmt = (version: string, count: number) => `Factr-I ${version} · Active sessions ${count}`

describe('systemStatusLine', () => {
  it('prints the count', () => {
    expect(systemStatusLine(fmt, { active_sessions: 2, version: '0.0.5' })).toBe('Factr-I 0.0.5 · Active sessions 2')
  })

  it('never prints undefined or null', () => {
    const line = systemStatusLine(fmt, { active_sessions: undefined, version: null })
    expect(line).not.toMatch(/undefined|null|NaN/)
    expect(line).toBe('Factr-I · Active sessions 0')
  })
})
