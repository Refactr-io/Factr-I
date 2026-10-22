import { compactNumber } from '@factr/shared'
import { useState } from 'react'

import { StableText } from '@/components/chat/stable-text'
import { useViewedInterval } from '@/hooks/use-viewed-interval'
import type { UsageStats } from '@/types/factr'

export function formatDuration(elapsedMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000))
  const seconds = totalSeconds % 60
  const minutes = Math.floor(totalSeconds / 60) % 60
  const hours = Math.floor(totalSeconds / 3600)
  const ss = String(seconds).padStart(2, '0')
  const mm = String(minutes).padStart(2, '0')

  return hours > 0 ? `${hours}:${mm}:${ss}` : `${minutes}:${ss}`
}

export function compactPath(path: string, max = 44): string {
  const trimmed = path.trim()

  if (trimmed.length <= max) {
    return trimmed
  }

  const segments = trimmed.split('/').filter(Boolean)

  if (segments.length < 2) {
    return `…${trimmed.slice(-(max - 1))}`
  }

  const tail = segments.slice(-2).join('/')

  return tail.length + 2 >= max ? `…${tail.slice(-(max - 1))}` : `…/${tail}`
}

export function contextBar(percent: number | undefined, width = 10): string {
  const bounded = Math.max(0, Math.min(100, percent ?? 0))
  const filled = Math.round((bounded / 100) * width)

  return `${'█'.repeat(filled)}${'░'.repeat(width - filled)}`
}

export function usageContextLabel(usage: UsageStats): string {
  if (usage.context_max) {
    return `${usage.context_estimated ? '~' : ''}${compactNumber(usage.context_used ?? 0)}/${compactNumber(usage.context_max)}`
  }

  return usage.total > 0 ? `${compactNumber(usage.total)} tok` : ''
}

export function contextBarLabel(usage: UsageStats): string {
  if (!usage.context_max) {
    return ''
  }

  const pct = Math.max(0, Math.min(100, Math.round(usage.context_percent ?? 0)))

  return `[${contextBar(usage.context_percent)}] ${usage.context_estimated ? '~' : ''}${pct}%`
}

/** `87%` for a reported hit rate; '' when the backend omitted it (no cache
 *  reads yet, or a provider that doesn't report them). The backend already
 *  clamps and rounds, so this only guards a malformed/absent field. */
export function cacheHitLabel(usage: UsageStats): string {
  const pct = usage.cache_hit_pct

  return typeof pct === 'number' && Number.isFinite(pct) ? `${Math.round(pct)}%` : ''
}

/** Dollars for a priced route: `$0.04`, `<$0.01`; '' when the route could not be priced. */
export function costLabel(cost: number | undefined): string {
  if (typeof cost !== 'number' || !Number.isFinite(cost) || cost <= 0) {
    return ''
  }

  return cost < 0.01 ? '<$0.01' : `$${cost < 10 ? cost.toFixed(2) : cost.toFixed(1)}`
}

/** The last turn at a glance: `12.3k in (9.1k cached) · 1.2k out · $0.04`; '' before the first call. The
 *  cached share counts inside `in`, so cached plus uncached always adds up. */
export function turnUsageLabel(usage: UsageStats): string {
  const turn = usage.turn

  if (!turn || turn.calls <= 0) {
    return ''
  }

  // Anthropic reports cache tokens beside the prompt, OpenAI inside it: show the full prompt either way.
  const prompt = turn.cache_read > turn.input ? turn.input + turn.cache_read + turn.cache_write : turn.input

  const parts = [
    turn.cache_read > 0
      ? `${compactNumber(prompt)} in (${compactNumber(turn.cache_read)} cached)`
      : `${compactNumber(prompt)} in`
  ]

  parts.push(`${compactNumber(turn.output)} out`)

  const cost = costLabel(turn.cost_usd)

  if (cost) {
    parts.push(cost)
  }

  return parts.join(' · ')
}

/** `34% left` for the most constrained provider limit, with the reset when it said: `34% left · 20s`. */
export function rateLimitLabel(usage: UsageStats): string {
  const limit = usage.rate_limit

  if (!limit || !Number.isFinite(limit.remaining_pct)) {
    return ''
  }

  const reset =
    typeof limit.reset_in_secs === 'number' && limit.reset_in_secs > 0 ? formatReset(limit.reset_in_secs) : ''

  return `${Math.round(limit.remaining_pct)}% left${reset ? ` · ${reset}` : ''}`
}

function formatReset(secs: number): string {
  if (secs < 90) {
    return `${Math.round(secs)}s`
  }

  if (secs < 5400) {
    return `${Math.round(secs / 60)}m`
  }

  return `${Math.round(secs / 3600)}h`
}

/** `42 t/s` for the rolling throughput; '' before the first completed call. */
export function tokensPerSecondLabel(usage: UsageStats): string {
  const tps = usage.avg_tps

  return typeof tps === 'number' && Number.isFinite(tps) && tps > 0 ? `${Math.round(tps)} t/s` : ''
}

export function LiveDuration({ since }: { since: number | null | undefined }) {
  const [now, setNow] = useState(() => Date.now())

  useViewedInterval(() => setNow(Date.now()), 1000, Boolean(since))

  if (!since) {
    return null
  }

  return <StableText>{formatDuration(now - since)}</StableText>
}
