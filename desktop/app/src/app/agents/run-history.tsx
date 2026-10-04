import { type ReactNode, useEffect, useRef, useState } from 'react'

import { factrApi, getApiRequestProfile, profileScoped } from '@/api/client'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { requestGatewayForProfile } from '@/store/gateway'

import { RunHistoryFilters, RunHistoryStats } from './run-history-stats'
import { AlertsPanel, FactsPanel, MemoryAuditPanel, SessionsPanel } from './run-history-views'

interface Run {
  id: string
  session_id: string
  parent_id: string | null
  root_id: string
  kind: string
  title: string | null
  model: string
  provider: string
  status: string
  started_at_ms: number
  ended_at_ms: number | null
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_write_tokens: number
  cost_usd: number | null
  unpriced_calls: number
  error: string | null
  no_model_call?: boolean
  replay_of?: string | null
  /** observability's fact_turn.outcome vocabulary (docs/OBSERVABILITY.md's mapping
   *  table): 'wedged' is derived live (open past an hour), never stored. */
  outcome?: 'ok' | 'failed' | 'no_model_call' | 'cancelled' | 'budget_stopped' | 'wedged' | 'running'
  span_coverage?: 'none' | 'partial' | 'full'
  /** Time to first streamed token, from the run's own start. */
  ttft_ms?: number | null
  step_count?: number
  tools_called?: number
  trigger?: 'desktop' | 'schedule'
}

/** observability.approvals columns. */
interface ApprovalRow {
  id: number
  session_id: string
  tool_name: string
  command_preview: string
  option_id: string
  approver: string
  approver_via: string
  decided_at_ms: number
}

interface BudgetPayload {
  spend_usd: number
  budget_daily_usd: number | null
  over_budget: boolean
}

interface Span {
  id: string
  kind: string
  name: string
  status: string
  started_at_ms: number
  ended_at_ms: number | null
  input_tokens: number
  output_tokens: number
  cost_usd: number | null
  error: string | null
  input: string | null
  output: string | null
}

const money = (value: number | null) => (value === null ? 'unpriced' : value === 0 ? '$0' : `$${value.toFixed(4)}`)
const runMoney = (run: Run) => (run.unpriced_calls > 0 ? 'unpriced' : money(run.cost_usd))
const duration = (start: number, end: number | null) =>
  `${((Math.max(start, end ?? Date.now()) - start) / 1000).toFixed(1)}s`
const time = (value: number) => new Date(value).toLocaleString()

const EASE_OUT = 'cubic-bezier(0.16, 1, 0.3, 1)'

/** Motion only ever moves content that is already on screen; nothing starts
 *  hidden, so if an animation never runs the view is simply static. */
const motionAllowed = (el: Element | null): el is HTMLElement =>
  el instanceof HTMLElement &&
  typeof el.animate === 'function' &&
  !window.matchMedia?.('(prefers-reduced-motion: reduce)').matches

/** Detail pane settles in when a different run is selected. */
function useSettle(key: string | null) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (key && motionAllowed(ref.current)) {
      ref.current.animate([{ transform: 'translateY(4px)' }, { transform: 'none' }], {
        duration: 220,
        easing: EASE_OUT
      })
    }
  }, [key])

  return ref
}

/** Timeline bars sweep in once per run (not on every 2 s refresh), revealed
 *  with a clip so their rounded ends never distort mid-animation. */
function useBarSweep(key: string | null, ready: boolean) {
  const ref = useRef<HTMLOListElement>(null)

  useEffect(() => {
    // A hidden window may never tick the animation, and the sweep starts empty.
    if (!key || !ready || document.hidden || !motionAllowed(ref.current)) {
      return
    }

    const sweeps = [...ref.current.querySelectorAll<HTMLElement>('[data-bar-fill]')].map((fill, index) =>
      fill.animate([{ clipPath: 'inset(0 100% 0 0)' }, { clipPath: 'inset(0 0 0 0)' }], {
        delay: index * 40,
        duration: 380,
        easing: EASE_OUT,
        fill: 'backwards'
      })
    )

    // The bars must never stay empty: if the animation stalls, jump to the end.
    const settle = () => sweeps.forEach(sweep => sweep.playState !== 'finished' && sweep.finish())
    const guard = window.setTimeout(settle, 1000)

    return () => {
      window.clearTimeout(guard)
      settle()
    }
  }, [key, ready])

  return ref
}

/** Status reads by meaning, not a chip: failures in the destructive tone,
 *  live work at full contrast, finished work quiet. */
const statusTone = (status: string) =>
  status === 'failed' || status === 'error'
    ? 'text-destructive'
    : status === 'running' || status === 'queued' || status === 'spawned'
      ? 'font-medium text-foreground'
      : status === 'interrupted'
        ? 'text-(--ui-text-secondary)'
        : 'text-(--ui-text-tertiary)'

/** A run stuck in `status='running'` past observability's wedge threshold — see
 *  operations::wedged_count. Distinct from an ordinary failure: nothing
 *  errored, the run simply never reached a terminal state. */
const isWedged = (run: Run) => run.outcome === 'wedged'

const bar = (span: Span, run: Run) => {
  const total = Math.max(1, (run.ended_at_ms ?? Date.now()) - run.started_at_ms)
  const left = Math.max(0, Math.min(99, ((span.started_at_ms - run.started_at_ms) / total) * 100))
  const width = Math.max(
    1,
    Math.min(100 - left, (((span.ended_at_ms ?? Date.now()) - span.started_at_ms) / total) * 100)
  )

  return { left: `${left}%`, width: `${width}%` }
}

/** The run ledger belongs to the active Factr backend; older Factr backends show a clear error. */
export function RunHistory() {
  const [runs, setRuns] = useState<Run[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [spans, setSpans] = useState<Span[]>([])
  const [loadedFor, setLoadedFor] = useState<string | null>(null)
  const [content, setContent] = useState<{ input: string | null; output: string | null } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [stopping, setStopping] = useState(false)
  const [captureContent, setCaptureContent] = useState(false)
  const [droppedEvents, setDroppedEvents] = useState(0)
  const [budget, setBudget] = useState<BudgetPayload | null>(null)
  const [statusFilter, setStatusFilter] = useState('')
  const [kindFilter, setKindFilter] = useState('')
  const [search, setSearch] = useState('')
  const [sideTab, setSideTab] = useState<'runs' | 'sessions' | 'approvals' | 'alerts' | 'facts' | 'memory'>('runs')
  const [outcomeFilter, setOutcomeFilter] = useState('')
  const [sessionFilter, setSessionFilter] = useState('')
  const [approvals, setApprovals] = useState<ApprovalRow[]>([])
  const [actionBusy, setActionBusy] = useState(false)

  useEffect(() => {
    let live = true
    const params = new URLSearchParams({ limit: '200' })

    if (statusFilter) {
      params.set('status', statusFilter)
    }

    if (kindFilter) {
      params.set('kind', kindFilter)
    }

    if (search.trim()) {
      params.set('q', search.trim())
    }

    if (outcomeFilter) {
      params.set('outcome', outcomeFilter)
    }

    if (sessionFilter) {
      params.set('session', sessionFilter)
    }

    const load = async () => {
      try {
        const result = await factrApi<{
          runs: Run[]
          capture_content: boolean
          dropped_events: number
          budget?: BudgetPayload
        }>({
          ...profileScoped(),
          path: `/api/factr/observability/runs?${params}`
        })

        if (!live) {
          return
        }
        setRuns(result.runs)
        setCaptureContent(result.capture_content)
        setDroppedEvents(result.dropped_events)

        if (result.budget) {
          setBudget(result.budget)
        }
        setSelected(current => current ?? result.runs[0]?.id ?? null)
        setError(null)
      } catch (cause) {
        if (live) {
          setError(cause instanceof Error ? cause.message : 'Run history is unavailable')
        }
      }
    }

    void load()
    const timer = window.setInterval(() => void load(), 2000)

    return () => {
      live = false
      window.clearInterval(timer)
    }
  }, [statusFilter, kindFilter, search, outcomeFilter, sessionFilter])

  useEffect(() => {
    if (sideTab !== 'approvals') {
      return
    }
    let live = true

    const load = async () => {
      try {
        const result = await factrApi<{ approvals: ApprovalRow[] }>({
          ...profileScoped(),
          path: '/api/factr/observability/approvals?limit=200'
        })

        if (live) {
          setApprovals(result.approvals)
        }
      } catch {
        /* optional */
      }
    }

    void load()
    const timer = window.setInterval(() => void load(), 5000)

    return () => {
      live = false
      window.clearInterval(timer)
    }
  }, [sideTab])

  useEffect(() => {
    if (!selected) {
      return
    }
    let live = true
    // A new selection starts empty; re-clicking the open run keeps its timeline.
    setSpans([])
    setContent(null)

    const load = async () => {
      try {
        const detail = await factrApi<{
          spans: Span[]
          content: { input: string | null; output: string | null } | null
        }>({
          ...profileScoped(),
          path: `/api/factr/observability/run?id=${encodeURIComponent(selected)}`
        })

        if (live) {
          setSpans(detail.spans)
          setContent(detail.content)
          setLoadedFor(selected)
        }
      } catch (cause) {
        if (live) {
          setError(cause instanceof Error ? cause.message : 'Could not load run')
        }
      }
    }

    void load()
    const timer = window.setInterval(() => void load(), 2000)

    return () => {
      live = false
      window.clearInterval(timer)
    }
  }, [selected])

  const settleRef = useSettle(selected)
  const barsRef = useBarSweep(selected, spans.length > 0)
  const run = runs.find(item => item.id === selected)
  const root = run ? runs.find(item => item.id === run.root_id) : null

  const byId = new Map(runs.map(item => [item.id, item]))

  const groupedRuns = [...runs].sort((a, b) => {
    const groupTime = (item: Run) => byId.get(item.root_id)?.started_at_ms ?? item.started_at_ms

    return (
      groupTime(b) - groupTime(a) ||
      Number(Boolean(a.parent_id)) - Number(Boolean(b.parent_id)) ||
      a.started_at_ms - b.started_at_ms
    )
  })

  const promote = async () => {
    if (!run) {
      return
    }
    setActionBusy(true)

    try {
      await factrApi({
        ...profileScoped(),
        method: 'POST',
        path: '/api/factr/observability/promote',
        body: { run_id: run.id }
      })
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Promote failed')
    } finally {
      setActionBusy(false)
    }
  }

  const replay = async () => {
    if (!run) {
      return
    }
    setActionBusy(true)

    try {
      await factrApi({
        ...profileScoped(),
        method: 'POST',
        path: '/api/factr/observability/replay',
        body: { run_id: run.id }
      })
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Replay failed (needs content capture and a running engine)')
    } finally {
      setActionBusy(false)
    }
  }

  const stop = async () => {
    if (!run) {
      return
    }
    setStopping(true)

    try {
      await requestGatewayForProfile(getApiRequestProfile() ?? 'default', 'session.interrupt', {
        session_id: run.session_id
      })
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Could not stop run')
    } finally {
      setStopping(false)
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 sm:flex-row">
      <div
        aria-label="Runs"
        className="flex max-h-[38%] min-h-0 w-full shrink-0 flex-col overflow-y-auto border-b border-(--ui-stroke-tertiary) pb-3 sm:max-h-none sm:w-64 sm:border-r sm:border-b-0 sm:pr-3 sm:pb-0"
      >
        <RunHistoryStats budgetFromList={budget} />
        <div className="flex flex-wrap gap-1 px-2 pb-2">
          {TABS.map(([id, label]) => (
            <button
              className={cn(tabClass, sideTab === id && tabActive)}
              key={id}
              onClick={() => setSideTab(id)}
              type="button"
            >
              {label}
            </button>
          ))}
        </div>
        {sideTab === 'runs' && (
          <RunHistoryFilters
            kind={kindFilter}
            onKind={setKindFilter}
            onOutcome={setOutcomeFilter}
            onQ={setSearch}
            onSession={() => setSessionFilter('')}
            onStatus={setStatusFilter}
            outcome={outcomeFilter}
            q={search}
            session={sessionFilter}
            status={statusFilter}
          />
        )}
        <p className="px-2 pb-2 text-xs text-(--ui-text-tertiary)">
          {captureContent ? 'Content capture on' : 'Content capture off'}
        </p>
        {droppedEvents > 0 && (
          <p className="px-2 py-1 text-sm text-destructive" role="alert">
            {droppedEvents} events missed
          </p>
        )}
        {error && (
          <p className="px-2 py-1 text-sm text-destructive" role="alert">
            {error}
          </p>
        )}
        {sideTab === 'sessions' && (
          <SessionsPanel
            onOpen={id => {
              setSessionFilter(id)
              setSideTab('runs')
            }}
          />
        )}
        {sideTab === 'alerts' && <AlertsPanel />}
        {sideTab === 'facts' && <FactsPanel />}
        {sideTab === 'memory' && <MemoryAuditPanel />}
        {sideTab === 'approvals' && (
          <ul className="flex flex-col gap-1 px-2">
            {approvals.length === 0 && (
              <li className="text-sm text-(--ui-text-secondary)">No approval decisions yet.</li>
            )}
            {approvals.map(row => (
              <li className="rounded-md bg-(--ui-widget-surface-background) p-2 text-xs" key={row.id}>
                <div className="font-medium text-foreground">
                  {row.tool_name} · {row.option_id}
                </div>
                <div className="text-(--ui-text-tertiary)">
                  {row.approver} ({row.approver_via}) · {time(row.decided_at_ms)}
                </div>
                <div className="mt-1 truncate text-(--ui-text-secondary)">{row.command_preview}</div>
              </li>
            ))}
          </ul>
        )}
        {sideTab === 'runs' && runs.length === 0 && !error && (
          <p className="px-2 py-1 text-sm text-(--ui-text-secondary)">No runs yet.</p>
        )}
        {sideTab === 'runs' && (
          <div className="flex flex-col gap-0.5">
            {groupedRuns.map(item => (
              <button
                aria-current={item.id === selected ? 'true' : undefined}
                className={cn(
                  'row-hover w-full rounded-md py-2 pr-2 text-left focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring',
                  item.parent_id ? 'pl-6' : 'pl-2',
                  item.id === selected && 'bg-(--ui-row-active-background)'
                )}
                key={item.id}
                onClick={() => setSelected(item.id)}
                type="button"
              >
                <span className="flex items-center justify-between gap-2 text-sm font-medium text-foreground">
                  <span className="truncate">{item.parent_id ? '↳ Agent' : item.title || runLabel(item.kind)}</span>
                  <span className={cn('shrink-0 text-xs', statusTone(listState(item)))}>
                    {listState(item).replace('_', ' ')}
                  </span>
                </span>
                <span className="mt-0.5 block truncate text-xs text-(--ui-text-tertiary)">
                  {kindName(item.kind)}
                  {item.trigger === 'schedule' ? ' · scheduled' : ''} ·{' '}
                  {(item.input_tokens + item.output_tokens).toLocaleString()} tokens · {runMoney(item)} ·{' '}
                  {time(item.started_at_ms)}
                  {item.no_model_call && <span className="text-amber-600 dark:text-amber-400"> · no model call</span>}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
      {run ? (
        <div aria-label="Run detail" className="min-h-0 min-w-0 flex-1 overflow-y-auto sm:pl-1" ref={settleRef}>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold text-foreground">
                {run.title || (run.parent_id ? 'Agent' : runLabel(run.kind))} ·{' '}
                <span className={statusTone(listState(run))}>{listState(run).replace('_', ' ')}</span>
              </h3>
              <p className="mt-1 text-xs text-(--ui-text-tertiary)">
                {kindName(run.kind)} · {time(run.started_at_ms)} · {run.provider} / {run.model}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              {run.status === 'complete' && (
                <>
                  <Button disabled={actionBusy} onClick={() => void promote()} size="sm" variant="secondary">
                    Promote to eval
                  </Button>
                  <Button disabled={actionBusy} onClick={() => void replay()} size="sm" variant="secondary">
                    Replay
                  </Button>
                </>
              )}
              {(run.status === 'running' || run.status === 'queued' || run.status === 'spawned') && (
                <Button disabled={stopping} onClick={() => void stop()} size="sm" variant="secondary">
                  {stopping ? 'Stopping…' : 'Stop run'}
                </Button>
              )}
            </div>
          </div>
          {run.no_model_call && run.status === 'complete' && !run.error && (
            <p
              className="mt-3 rounded-md bg-amber-500/10 px-3 py-2 text-sm text-amber-800 dark:text-amber-200"
              role="status"
            >
              This run finished without any model calls — the reply may be empty or skipped.
            </p>
          )}
          {isWedged(run) && (
            <p className="mt-3 rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive" role="alert">
              {run.status === 'interrupted'
                ? 'The engine stopped while this run was in progress. Nothing will retry it.'
                : 'This run has been open for over an hour without finishing. Nothing will retry it.'}
            </p>
          )}
          {run.replay_of && <p className="mt-2 text-xs text-(--ui-text-tertiary)">Replay of run {run.replay_of}</p>}
          <div className="mt-5 grid grid-cols-1 gap-4 rounded-lg bg-(--ui-widget-surface-background) p-4 text-sm sm:grid-cols-3">
            <Stat label="Duration" value={duration(run.started_at_ms, run.ended_at_ms)} />
            <Stat
              detail={
                <>
                  {run.input_tokens.toLocaleString()} in · {run.output_tokens.toLocaleString()} out
                  <br />
                  {run.cache_read_tokens.toLocaleString()} cache read · {run.cache_write_tokens.toLocaleString()} write
                </>
              }
              label="Tokens"
              value={(run.input_tokens + run.output_tokens).toLocaleString()}
            />
            <Stat label="API cost" value={runMoney(run)} />
            <Stat label="First token" value={run.ttft_ms == null ? '-' : `${(run.ttft_ms / 1000).toFixed(1)}s`} />
            <Stat label="Model calls" value={String(run.step_count ?? 0)} />
            <Stat label="Tool calls" value={String(run.tools_called ?? 0)} />
          </div>
          {root && root.id !== run.id && (
            <Button className="mt-4" onClick={() => setSelected(root.id)} size="sm" variant="text">
              View parent run
            </Button>
          )}
          {run.error && (
            <p className="mt-4 rounded-md bg-destructive/10 p-3 text-sm text-destructive" role="alert">
              {run.error}
            </p>
          )}
          <h4 className="mt-6 text-sm font-semibold text-foreground">Timeline</h4>
          {loadedFor === selected && spans.length === 0 && (
            <p className="mt-2 text-sm text-(--ui-text-secondary)">No events recorded.</p>
          )}
          <ol className="mt-2 flex flex-col gap-1.5" ref={barsRef}>
            {spans.map(span => (
              <li
                className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 rounded-md bg-(--ui-widget-surface-background) px-3 py-2.5"
                key={span.id}
              >
                <span className="min-w-0 truncate text-sm text-foreground">
                  {span.kind === 'chat' ? 'Model call' : span.name}
                </span>
                <span className="text-xs tabular-nums text-(--ui-text-tertiary)">
                  {duration(span.started_at_ms, span.ended_at_ms)}
                </span>
                <span className="text-xs text-(--ui-text-tertiary)">
                  <span className={statusTone(span.status)}>{span.status}</span> ·{' '}
                  {(span.input_tokens + span.output_tokens).toLocaleString()} tokens ·{' '}
                  {span.kind === 'execute_tool' ? '$0' : money(span.cost_usd)}
                </span>
                <span aria-hidden className="relative col-span-2 mt-2 block h-1.5 rounded-xs bg-(--ui-bg-tertiary)">
                  <span
                    className="absolute inset-y-0 block min-w-1.5 rounded-xs bg-foreground/70"
                    data-bar-fill
                    style={bar(span, run)}
                  />
                </span>
                {span.error && <span className="col-span-2 mt-1 text-xs text-destructive">{span.error}</span>}
                {(span.input || span.output) && (
                  <details className="col-span-2 mt-1 text-xs">
                    <summary className="cursor-pointer text-(--ui-text-secondary)">Content</summary>
                    <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-(--ui-bg-tertiary) p-2">
                      {span.input}
                      {span.output ? `\n${span.output}` : ''}
                    </pre>
                  </details>
                )}
              </li>
            ))}
          </ol>
          {(content?.input || content?.output) && (
            <details className="mt-5 text-sm">
              <summary className="cursor-pointer text-(--ui-text-secondary)">Turn content</summary>
              <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-(--ui-widget-surface-background) p-3 text-xs">
                {content.input}
                {content.output ? `\n${content.output}` : ''}
              </pre>
            </details>
          )}
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center p-6 text-sm text-(--ui-text-secondary)">
          Select a run
        </div>
      )}
    </div>
  )
}

// `invoke_agent` is the OTel GenAI operation name for a chat turn; show it as a chat.
function kindName(kind: string) {
  return kind === 'invoke_agent' ? 'chat' : kind
}

function runLabel(kind: string) {
  return kind === 'invoke_agent' ? 'Chat' : kind
}

/** Wedged and cancelled are outcomes, not statuses: show the outcome once a run
 *  is no longer plainly running or done. */
const listState = (run: Run) =>
  run.outcome && !['ok', 'running', 'no_model_call'].includes(run.outcome) ? run.outcome : run.status

const TABS = [
  ['runs', 'Runs'],
  ['sessions', 'Sessions'],
  ['approvals', 'Approvals'],
  ['alerts', 'Alerts'],
  ['facts', 'Facts'],
  ['memory', 'Memory']
] as const

const tabClass = 'rounded-md px-2 py-1 text-xs text-(--ui-text-secondary)'
const tabActive = 'bg-(--ui-row-active-background) font-medium text-foreground'

function Stat({ detail, label, value }: { detail?: ReactNode; label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-(--ui-text-tertiary)">{label}</div>
      <div className="mt-1 text-base font-medium tabular-nums text-foreground">{value}</div>
      {detail && <div className="mt-1 text-xs tabular-nums text-(--ui-text-tertiary)">{detail}</div>}
    </div>
  )
}
