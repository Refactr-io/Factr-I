import { describe, expect, it } from 'vitest'

import { GREETINGS, momentsAt, pickGreeting } from './intro'

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

  it('never claims a time, a season or the weather outside its moment', () => {
    // Anything that names a time of day is tagged to it, and nothing guesses the weather or the season.
    const untagged = GREETINGS.filter(([, moment]) => !moment).map(([text]) => text.toLowerCase())

    for (const text of untagged) {
      expect(text).not.toMatch(/morning|afternoon|evening|night|midnight|sunrise|moon|star|weekend|rain|snow|sun\b|summer|winter/)
    }

    expect(GREETINGS.flatMap(([text]) => (/rain|snow|summer|winter/i.test(text) ? [text] : []))).toEqual([])
  })

  it('has at least four phrases for every part of the day', () => {
    for (const moment of ['morning', 'afternoon', 'evening', 'night']) {
      expect(GREETINGS.filter(([, tag]) => tag === moment).length).toBeGreaterThanOrEqual(4)
    }
  })
})
