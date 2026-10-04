import { useStore } from '@nanostores/react'
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'

import { useGatewayRequest } from '@/app/gateway/hooks/use-gateway-request'
import { PageLoader } from '@/components/page-loader'
import { Button } from '@/components/ui/button'
import { getHarness, rollbackHarness } from '@/factr'
import { useI18n } from '@/i18n'
import { queryClient } from '@/lib/query-client'
import { relativeTime } from '@/lib/time'
import { notifyError } from '@/store/notifications'
import { $activeSessionId } from '@/store/session'

import { CapRow, DetailColumn, ListColumn, ListStrip, ListStripButton, MasterDetail } from '../../master-detail'
import { PanelEmpty } from '../../overlays/panel'
import { DetailHeader } from '../primitives'

export const HARNESS_QUERY_KEY = ['harness'] as const

/** What factr-learn has taught the agent: the learned instructions injected into new
 *  chats, the changesets that made them (each undoable), and a manual "refine
 *  now" that runs the same /refine path as the slash command. */
export function HarnessTab() {
  const { t } = useI18n()
  const h = t.skills.harness
  const { requestGateway } = useGatewayRequest()
  const sessionId = useStore($activeSessionId)
  const [selected, setSelected] = useState<string | null>(null)
  const [busy, setBusy] = useState<'refine' | string | null>(null)
  const { data, isError, error } = useQuery({ queryKey: HARNESS_QUERY_KEY, queryFn: getHarness, staleTime: 0 })

  if (isError) {
    return (
      <PanelEmpty description={error instanceof Error ? error.message : undefined} icon="error" title={h.loadFailed} />
    )
  }

  if (!data) {
    return <PageLoader label={t.skills.loading} />
  }

  const entries = data.entries
  const active = entries.find(e => e.id === selected) ?? entries[0] ?? null
  const reload = () => queryClient.invalidateQueries({ queryKey: HARNESS_QUERY_KEY })

  async function refineNow() {
    if (!sessionId) {
      return
    }

    setBusy('refine')

    try {
      await requestGateway('slash.exec', { command: 'refine', session_id: sessionId })
      await reload()
    } catch (err) {
      notifyError(err, h.refineFailed)
    } finally {
      setBusy(null)
    }
  }

  async function rollback(id: string) {
    setBusy(id)

    try {
      await rollbackHarness(id)
      await reload()
    } catch (err) {
      notifyError(err, h.rollbackFailed)
    } finally {
      setBusy(null)
    }
  }

  return (
    <MasterDetail resizeId="capabilities-split" split="wide">
      <ListColumn
        header={
          <ListStrip
            left={
              <ListStripButton disabled={!sessionId || busy !== null} onClick={() => void refineNow()}>
                {busy === 'refine' ? h.refining : h.refineNow}
              </ListStripButton>
            }
            right={
              !sessionId ? (
                <span className="text-[0.625rem] text-muted-foreground/90">{h.openChatToRefine}</span>
              ) : undefined
            }
          />
        }
      >
        {entries.map(entry => (
          <CapRow
            action={<span />}
            active={active?.id === entry.id}
            enabled
            key={entry.id}
            meta={entry.scope}
            onSelect={() => setSelected(entry.id)}
            subtitle={entry.path || entry.source}
            title={entry.title}
          />
        ))}
      </ListColumn>
      <DetailColumn>
        {active ? (
          <>
            <DetailHeader description={relativeTime(active.timestamp * 1000)} title={active.title} />
            <p className="whitespace-pre-wrap text-sm leading-relaxed text-(--ui-text-secondary)">{active.content}</p>
          </>
        ) : (
          <DetailHeader description={h.emptyDesc} title={h.emptyTitle} />
        )}
        {data.changesets.length > 0 && (
          <section className="grid gap-1">
            <h4 className="text-[0.75rem] font-semibold text-(--ui-text-secondary)">{h.changes}</h4>
            {data.changesets.map(cs => (
              <div className="flex items-baseline gap-3 py-1" key={cs.id}>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[0.8125rem] text-foreground/85">{cs.summary}</span>
                  <span className="block text-[0.625rem] text-muted-foreground/90">
                    {relativeTime(cs.timestamp * 1000)} · {h.editsCount(cs.edits)}
                  </span>
                </span>
                {cs.rolledBack || cs.isRollback ? (
                  <span className="text-[0.6875rem] text-muted-foreground/90">{cs.isRollback ? '' : h.rolledBack}</span>
                ) : (
                  <Button disabled={busy !== null} onClick={() => void rollback(cs.id)} size="xs" variant="textStrong">
                    {h.rollback}
                  </Button>
                )}
              </div>
            ))}
          </section>
        )}
      </DetailColumn>
    </MasterDetail>
  )
}
