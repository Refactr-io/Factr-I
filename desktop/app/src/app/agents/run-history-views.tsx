import { useEffect, useState } from 'react'

import { factrApi, profileScoped } from '@/api/client'

/** The rest of observability's dashboard views, served by the engine's
 *  /api/factr/observability/{sessions,alerts,facts,memory-audit}. Each
 *  panel only polls while its tab is mounted. */
function usePoll<T>(path: string, ms = 5000) {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true

    const load = async () => {
      try {
        const body = await factrApi<T>({ ...profileScoped(), path })

        if (live) {
          setData(body)
          setError(null)
        }
      } catch (cause) {
        if (live) {
          setError(cause instanceof Error ? cause.message : 'Unavailable')
        }
      }
    }

    void load()
    const timer = window.setInterval(() => void load(), ms)

    return () => {
      live = false
      window.clearInterval(timer)
    }
  }, [path, ms])

  return { data, error }
}

const row = 'rounded-md bg-(--ui-widget-surface-background) p-2 text-xs'
const quiet = 'text-(--ui-text-tertiary)'
const when = (value: number) => new Date(value).toLocaleString()
const usd = (value: number | null) => (value === null ? 'unpriced' : `$${value.toFixed(4)}`)

const outcomeTone = (outcome: string) =>
  outcome === 'failed' || outcome === 'wedged'
    ? 'text-destructive'
    : outcome === 'running'
      ? 'font-medium text-foreground'
      : quiet

function Empty({ error, text }: { error: string | null; text: string }) {
  return error ? (
    <p className="px-2 text-sm text-destructive" role="alert">
      {error}
    </p>
  ) : (
    <p className="px-2 text-sm text-(--ui-text-secondary)">{text}</p>
  )
}

interface SessionRow {
  session_id: string
  turns: number
  subagents: number
  failed: number
  cancelled: number
  wedged: number
  running: number
  input_tokens: number
  output_tokens: number
  cost_usd: number | null
  model: string
  provider: string
  trigger: string
  last_outcome: string
  last_started_at_ms: number
}

/** observability's sessions list: a session's turns rolled up, outcome first. */
export function SessionsPanel({ onOpen }: { onOpen: (sessionId: string) => void }) {
  const { data, error } = usePoll<{ sessions: SessionRow[] }>('/api/factr/observability/sessions?limit=100')

  return (
    <ul className="flex flex-col gap-1 px-2">
      {data?.sessions.length === 0 && <Empty error={error} text="No sessions yet." />}
      {!data && <Empty error={error} text="Loading…" />}
      {data?.sessions.map(item => (
        <li key={item.session_id}>
          <button className={`${row} row-hover w-full text-left`} onClick={() => onOpen(item.session_id)} type="button">
            <span className="flex items-center justify-between gap-2 font-medium text-foreground">
              <span className="truncate">{item.session_id}</span>
              <span className={outcomeTone(item.wedged > 0 ? 'wedged' : item.last_outcome)}>
                {item.wedged > 0 ? 'wedged' : item.last_outcome}
              </span>
            </span>
            <span className={`mt-0.5 block truncate ${quiet}`}>
              {item.trigger} · {item.provider} {item.model} · {item.turns} turn{item.turns === 1 ? '' : 's'}
              {item.subagents > 0 ? ` · ${item.subagents} subagent${item.subagents === 1 ? '' : 's'}` : ''}
              {item.failed > 0 ? ` · ${item.failed} failed` : ''}
              {item.cancelled > 0 ? ` · ${item.cancelled} cancelled` : ''}
            </span>
            <span className={`block truncate tabular-nums ${quiet}`}>
              {(item.input_tokens + item.output_tokens).toLocaleString()} tokens · {usd(item.cost_usd)} ·{' '}
              {when(item.last_started_at_ms)}
            </span>
          </button>
        </li>
      ))}
    </ul>
  )
}

interface AlertRow {
  id: string
  state: string
  severity: string
  title: string
  detail: string | null
  since_ms: number
}

/** observability's monitors page: firing alerts first, each with how long it has
 *  been in its state. */
export function AlertsPanel() {
  const { data, error } = usePoll<{ monitors: AlertRow[] }>('/api/factr/observability/alerts')
  const rows = [...(data?.monitors ?? [])].sort(
    (a, b) => Number(b.state === 'firing') - Number(a.state === 'firing') || a.id.localeCompare(b.id)
  )

  return (
    <ul className="flex flex-col gap-1 px-2">
      {rows.length === 0 && <Empty error={error} text="No monitor has been evaluated yet." />}
      {rows.map(item => (
        <li className={row} key={item.id}>
          <div className="flex items-center justify-between gap-2 font-medium text-foreground">
            <span>{item.title}</span>
            <span className={item.state === 'firing' ? 'text-destructive' : quiet}>{item.state.replace('_', ' ')}</span>
          </div>
          <div className={quiet}>
            {item.severity} · since {when(item.since_ms)}
          </div>
          {item.detail && <div className="mt-1 text-(--ui-text-secondary)">{item.detail}</div>}
        </li>
      ))}
    </ul>
  )
}

interface FactsBody {
  turn_outcomes: { model: string; outcome: string; turns: number }[]
  tools: {
    tool_name: string
    calls: number
    failed: number
    unjudged: number
    failure_rate_pct: number | null
    p95_ms: number | null
  }[]
}

/** observability's facts: turn outcomes per model, and tool calls with a failure
 *  rate over judged calls only (unjudged calls are counted, never scored). */
export function FactsPanel() {
  const { data, error } = usePoll<FactsBody>('/api/factr/observability/facts?days=7')

  if (!data) {
    return <Empty error={error} text="Loading…" />
  }

  return (
    <div className="flex flex-col gap-3 px-2">
      <p className={`text-xs ${quiet}`}>Last 7 days</p>
      <ul className="flex flex-col gap-1">
        {data.turn_outcomes.length === 0 && (
          <li className="text-sm text-(--ui-text-secondary)">No turns in this window.</li>
        )}
        {data.turn_outcomes.map(item => (
          <li className={`${row} flex justify-between gap-2`} key={`${item.model}:${item.outcome}`}>
            <span className="truncate text-foreground">{item.model}</span>
            <span className={`shrink-0 tabular-nums ${outcomeTone(item.outcome)}`}>
              {item.outcome.replace('_', ' ')} · {item.turns}
            </span>
          </li>
        ))}
      </ul>
      <ul className="flex flex-col gap-1">
        {data.tools.map(item => (
          <li className={row} key={item.tool_name}>
            <div className="flex justify-between gap-2 font-medium text-foreground">
              <span className="truncate">{item.tool_name}</span>
              <span className="tabular-nums">{item.calls}</span>
            </div>
            <div className={`tabular-nums ${quiet}`}>
              {item.failure_rate_pct === null ? 'no judged calls' : `${item.failure_rate_pct.toFixed(0)}% failed`}
              {item.unjudged > 0 ? ` · ${item.unjudged} unjudged` : ''}
              {item.p95_ms !== null ? ` · p95 ${(item.p95_ms / 1000).toFixed(1)}s` : ''}
            </div>
          </li>
        ))}
      </ul>
    </div>
  )
}

interface DeletionRow {
  id: number
  deleted_at_ms: number
  memory_id: string
  scope: string | null
  category: string | null
  length: number
  actor: string | null
  actor_via: string
}

/** observability's memory audit: what was deleted from memory. The text itself is not kept. */
export function MemoryAuditPanel() {
  const { data, error } = usePoll<{ deletions: DeletionRow[] }>('/api/factr/observability/memory-audit?limit=100')

  return (
    <ul className="flex flex-col gap-1 px-2">
      {data?.deletions.length === 0 && <Empty error={error} text="No memory has been deleted." />}
      {!data && <Empty error={error} text="Loading…" />}
      {data?.deletions.map(item => (
        <li className={row} key={item.id}>
          <div className="font-medium text-foreground">
            {item.scope ?? 'memory'} · {item.memory_id}
          </div>
          <div className={quiet}>
            {item.actor ?? 'unidentified'} · {when(item.deleted_at_ms)}
            {item.category ? ` · ${item.category}` : ''} · {item.length} chars
          </div>
        </li>
      ))}
    </ul>
  )
}
