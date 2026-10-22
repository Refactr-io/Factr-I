import { describe, expect, it } from 'vitest'

import type { UsageStats } from '@/types/factr'

import { costLabel, rateLimitLabel, turnUsageLabel } from './statusbar'

const base: UsageStats = { calls: 0, input: 0, output: 0, total: 0 }

describe('turnUsageLabel', () => {
  it('is empty before the first call of a turn', () => {
    expect(turnUsageLabel(base)).toBe('')
    expect(turnUsageLabel({ ...base, turn: { cache_read: 0, cache_write: 0, calls: 0, input: 0, output: 0 } })).toBe('')
  })

  it('shows prompt, the cached share and output, with a cost only when priced', () => {
    const turn = { cache_read: 9100, cache_write: 0, calls: 2, input: 12_300, output: 1200 }

    expect(turnUsageLabel({ ...base, turn })).toBe('12.3k in (9.1k cached) · 1.2k out')
    expect(turnUsageLabel({ ...base, turn: { ...turn, cost_usd: 0.04 } })).toBe(
      '12.3k in (9.1k cached) · 1.2k out · $0.04'
    )
  })

  it('adds the cache back into the prompt when the provider reports it beside the input', () => {
    const turn = { cache_read: 9000, cache_write: 500, calls: 1, input: 800, output: 100 }

    expect(turnUsageLabel({ ...base, turn })).toBe('10.3k in (9k cached) · 100 out')
  })

  it('omits the cached part when nothing was cached', () => {
    expect(turnUsageLabel({ ...base, turn: { cache_read: 0, cache_write: 0, calls: 1, input: 500, output: 50 } })).toBe(
      '500 in · 50 out'
    )
  })
})

describe('costLabel', () => {
  it('formats small, normal and large costs and hides unpriced ones', () => {
    expect(costLabel(undefined)).toBe('')
    expect(costLabel(0)).toBe('')
    expect(costLabel(0.004)).toBe('<$0.01')
    expect(costLabel(0.5)).toBe('$0.50')
    expect(costLabel(12.34)).toBe('$12.3')
  })
})

describe('rateLimitLabel', () => {
  it('shows what is left and the reset when the provider gave one', () => {
    const limit = { kind: 'tokens' as const, limit: 100, provider: 'p', remaining: 34, remaining_pct: 34 }

    expect(rateLimitLabel({ ...base, rate_limit: { ...limit, reset_in_secs: 20 } })).toBe('34% left · 20s')
    expect(rateLimitLabel({ ...base, rate_limit: { ...limit, reset_in_secs: 600 } })).toBe('34% left · 10m')
    expect(rateLimitLabel({ ...base, rate_limit: limit })).toBe('34% left')
    expect(rateLimitLabel(base)).toBe('')
  })
})
