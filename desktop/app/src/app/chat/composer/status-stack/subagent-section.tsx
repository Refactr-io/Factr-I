import { useState } from 'react'

import { SubagentRow } from '@/app/agents'
import { openSession } from '@/app/open-session'
import { ActivityTimerText } from '@/components/chat/activity-timer-text'
import { StatusRow } from '@/components/chat/status-row'
import { StatusSection } from '@/components/chat/status-section'
import { Button } from '@/components/ui/button'
import { Codicon } from '@/components/ui/codicon'
import { LatticeLoader } from '@/components/ui/lattice-loader'
import { useViewedInterval } from '@/hooks/use-viewed-interval'
import { useI18n } from '@/i18n'
import { useSessionSlice } from '@/lib/use-session-slice'
import { $subagentsBySession, type SubagentProgress } from '@/store/subagents'

import { SubagentControls } from './subagent-controls'
import { SubagentTranscript } from './subagent-transcript'

interface SubagentSectionProps {
  sessionId: string
}

/** A composer-local roster: never borrow the global Agents panel's scope. */
export function SubagentSection({ sessionId }: SubagentSectionProps) {
  const { t } = useI18n()
  const items = useSessionSlice($subagentsBySession, sessionId)
  const isLive = (item: SubagentProgress) => item.status === 'running' || item.status === 'queued'
  const live = items.filter(isLive)
  const [nowMs, setNowMs] = useState(Date.now)
  const [selected, setSelected] = useState<string | null>(null)
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const hasLive = live.length > 0

  useViewedInterval(() => setNowMs(Date.now()), 1000, hasLive)

  // Every child stays listed after it finishes, with its outcome, until the turn that spawned it is pruned.
  if (items.length === 0) {
    return null
  }

  const row = (item: SubagentProgress) => (
    <StatusRow
      expanded={selected === item.id}
      key={item.id}
      leading={
        isLive(item) ? (
          <LatticeLoader
            ariaLabel={item.status === 'queued' ? t.agents.queued : t.agents.running}
            className="text-(--ui-purple)"
          />
        ) : (
          <Codicon
            aria-label={item.status === 'completed' ? t.agents.done : t.agents.failed}
            className={item.status === 'completed' ? 'text-(--ui-text-tertiary)' : 'text-destructive'}
            name={item.status === 'completed' ? 'check' : 'error'}
            size="0.8rem"
          />
        )
      }
      onActivate={() => setSelected(selected === item.id ? null : item.id)}
      trailing={
        <ActivityTimerText
          className="shrink-0 text-[0.625rem]"
          seconds={
            isLive(item)
              ? Math.max(0, Math.floor((nowMs - item.startedAt) / 1000))
              : Math.max(0, Math.round(item.durationSeconds ?? (item.updatedAt - item.startedAt) / 1000))
          }
        />
      }
      trailingVisible
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-xs text-(--ui-text-primary)">{item.goal}</span>
        <span className="block truncate text-[0.6875rem] text-(--ui-text-tertiary)">
          {item.stream.at(-1)?.text ||
            item.summary ||
            (item.status === 'queued' ? t.agents.queued : isLive(item) ? t.agents.waitingActivity : '')}
        </span>
      </span>
    </StatusRow>
  )

  const detail = items.find(item => item.id === selected)

  return (
    <div className="composer-no-drag min-w-0" data-slot="composer-subagents">
      <StatusSection
        collapsedIndicator={
          hasLive ? (
            <LatticeLoader
              ariaLabel={live.some(item => item.status === 'running') ? t.agents.running : t.agents.queued}
              className="text-(--ui-purple)"
            />
          ) : (
            <Codicon className="text-(--ui-text-tertiary)" name="check" size="0.8rem" />
          )
        }
        icon={<Codicon className="text-(--ui-purple)" name="agent" size="0.8rem" />}
        label={t.statusStack.subagents(items.length)}
      >
        <div className="max-h-[25vh] overflow-y-auto overscroll-y-auto">{items.map(row)}</div>
        {detail && (
          <div
            className="status-subagent-detail max-h-[25vh] overflow-y-auto overscroll-y-auto pr-3 py-2"
            data-slot="composer-subagent-detail"
          >
            {isLive(detail) && (
              <SubagentControls
                key={`${sessionId}:${detail.id}`}
                sessionId={sessionId}
                setText={text => setDrafts(previous => ({ ...previous, [detail.id]: text }))}
                subagentId={detail.id}
                text={drafts[detail.id] ?? ''}
              />
            )}
            {detail.sessionId && (
              <Button onClick={() => openSession(detail.sessionId!, () => undefined, 'tab')} size="xs" variant="text">
                {t.statusStack.openSubagentChat}
              </Button>
            )}
            <SubagentRow node={{ ...detail, children: [] }} nowMs={nowMs} />
            <SubagentTranscript key={`tail:${sessionId}:${detail.id}`} sessionId={sessionId} subagentId={detail.id} />
          </div>
        )}
      </StatusSection>
    </div>
  )
}
