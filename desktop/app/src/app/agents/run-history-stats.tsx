import { useEffect, useState } from 'react'

import { factrApi, profileScoped } from '@/api/client'
import { cn } from '@/lib/utils'

interface MonitorSlice {
  kind: string
  count: number
  p95_ms: number | null
}

interface MonitorsBody {
  error_rate_pct: number
  silent_failures: number
  wedged_count?: number
  run_latency: MonitorSlice[]
}

interface BudgetBody {
  spend_usd: number
  budget_daily_usd: number | null
  over_budget: boolean
}

/** Compact Observability-style strip: latency, errors, silent failures, budget. */
export function RunHistoryStats({ budgetFromList }: { budgetFromList?: BudgetBody | null }) {
  const [monitors, setMonitors] = useState<MonitorsBody | null>(null)
  const [budget, setBudget] = useState<BudgetBody | null>(budgetFromList ?? null)

  useEffect(() => {
    let live = true

    const load = async () => {
      try {
        const [mon, bud] = await Promise.all([
          factrApi<MonitorsBody>({ ...profileScoped(), path: '/api/factr/observability/monitors?window=24h' }),
          budgetFromList
            ? Promise.resolve(budgetFromList)
            : factrApi<BudgetBody>({ ...profileScoped(), path: '/api/factr/observability/budget' })
        ])

        if (live) {
          setMonitors(mon)
          setBudget(bud)
        }
      } catch {
        /* strip is optional when backend is older */
      }
    }

    void load()
    const timer = window.setInterval(() => void load(), 15_000)

    return () => {
      live = false
      window.clearInterval(timer)
    }
  }, [budgetFromList])

  if (!monitors && !budget?.over_budget) {
    return null
  }

  const chatP95 = monitors?.run_latency?.find(r => r.kind === 'invoke_agent')?.p95_ms

  return (
    <div className="flex flex-col gap-1 border-b border-(--ui-stroke-tertiary) px-2 pb-2 text-xs text-(--ui-text-tertiary)">
      {budget?.over_budget && (
        <p className="rounded-md bg-amber-500/10 px-2 py-1 text-amber-700 dark:text-amber-300" role="status">
          Daily spend ${budget.spend_usd.toFixed(2)}
          {budget.budget_daily_usd != null ? ` / $${budget.budget_daily_usd.toFixed(2)} budget` : ''} — soft limit only
        </p>
      )}
      {monitors && (
        <p className="tabular-nums">
          24h · err {(monitors.error_rate_pct ?? 0).toFixed(1)}% · silent {monitors.silent_failures ?? 0}
          {monitors.wedged_count ? ` · wedged ${monitors.wedged_count}` : ''}
          {chatP95 != null ? ` · chat p95 ${(chatP95 / 1000).toFixed(1)}s` : ''}
        </p>
      )}
    </div>
  )
}

export function RunHistoryFilters({
  status,
  kind,
  q,
  outcome,
  session,
  onStatus,
  onKind,
  onQ,
  onOutcome,
  onSession
}: {
  status: string
  kind: string
  q: string
  outcome: string
  session: string
  onStatus: (v: string) => void
  onKind: (v: string) => void
  onQ: (v: string) => void
  onOutcome: (v: string) => void
  onSession: () => void
}) {
  return (
    <div className="flex flex-col gap-2 px-2 pb-2">
      <input
        aria-label="Search runs"
        className="w-full rounded-md border border-(--ui-stroke-tertiary) bg-(--ui-widget-surface-background) px-2 py-1 text-sm"
        onChange={e => onQ(e.target.value)}
        placeholder="Search title or model"
        value={q}
      />
      {session && (
        <button
          className="truncate rounded-md bg-(--ui-row-active-background) px-2 py-1 text-left text-xs text-foreground"
          onClick={onSession}
          type="button"
        >
          Session {session} (clear)
        </button>
      )}
      <select
        aria-label="Filter outcome"
        className={selectClass}
        onChange={e => onOutcome(e.target.value)}
        value={outcome}
      >
        <option value="">All outcomes</option>
        <option value="ok">ok</option>
        <option value="failed">failed</option>
        <option value="cancelled">cancelled</option>
        <option value="no_model_call">no model call</option>
        <option value="wedged">wedged</option>
        <option value="running">running</option>
      </select>
      <div className="flex gap-2">
        <select
          aria-label="Filter status"
          className={selectClass}
          onChange={e => onStatus(e.target.value)}
          value={status}
        >
          <option value="">All statuses</option>
          <option value="complete">complete</option>
          <option value="running">running</option>
          <option value="error">error</option>
          <option value="interrupted">interrupted</option>
        </select>
        <select aria-label="Filter kind" className={selectClass} onChange={e => onKind(e.target.value)} value={kind}>
          <option value="">All kinds</option>
          <option value="invoke_agent">chat</option>
          <option value="cron">cron</option>
        </select>
      </div>
    </div>
  )
}

const selectClass = cn(
  'min-w-0 flex-1 rounded-md border border-(--ui-stroke-tertiary) bg-(--ui-widget-surface-background) px-2 py-1 text-xs'
)
