import { describe, expect, it } from 'vitest'

import { momentsAt, pickGreeting } from './intro'

describe('empty chat greeting', () => {
  it('knows the part of the day and the weekend', () => {
    expect(momentsAt(new Date(2026, 9, 10, 8))).toEqual(['morning', 'weekend'])
    expect(momentsAt(new Date(2026, 9, 12, 14))).toEqual(['afternoon', 'weekday'])
    expect(momentsAt(new Date(2026, 9, 12, 19))).toEqual(['evening', 'weekday'])
    expect(momentsAt(new Date(2026, 9, 12, 2))).toEqual(['night', 'weekday'])
  })

  it('offers a night phrase at night and never a morning phrase', () => {
    const night = new Date(2026, 9, 12, 2)
    const seen = new Set(Array.from({ length: 200 }, (_, seed) => pickGreeting(seed, night)))

    expect(seen.has('Moonlit chat')).toBe(true)
    expect(seen.has('Sunrise session')).toBe(false)
    expect(seen.has('Weekend project?')).toBe(false)
  })

  it('offers the weekend line only on a weekend', () => {
    const saturday = new Date(2026, 9, 10, 15)
    const monday = new Date(2026, 9, 12, 15)

    expect(Array.from({ length: 300 }, (_, seed) => pickGreeting(seed, saturday)).includes('Weekend project?')).toBe(true)
    expect(Array.from({ length: 300 }, (_, seed) => pickGreeting(seed, monday)).includes('Weekend project?')).toBe(false)
  })
})
