import { useStore } from '@nanostores/react'
import { useCallback, useEffect, useState } from 'react'

import {
  type AgentImportGroup,
  type AgentImportReport,
  type AgentImportSource,
  detectAgentImports,
  GROUP_ORDER,
  groupCounts,
  runAgentImport
} from '@/api/agent-import'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle
} from '@/components/ui/dialog'
import { useI18n } from '@/i18n'
import { AlertTriangle } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $agentImportOpen, closeAgentImport } from '@/store/agent-import'
import { notifyError } from '@/store/notifications'
import { $activeGatewayProfile, normalizeProfileKey, refreshProfiles } from '@/store/profile'

type Step = 'pick' | 'result' | 'running'

interface Picked {
  agent: string
  counts: Record<string, number>
  label: string
  path: string
}

const rowClass =
  'flex w-full items-center justify-between gap-3 rounded-lg border px-3 py-2 text-left transition-colors duration-150 ease-out focus-visible:outline-2 focus-visible:outline-ring'

/**
 * Import another agent's setup into the active profile: pick what was found (or any folder), choose what to
 * bring, and read what happened. One dialog for Claude Code, Codex, Hermes and OpenCode.
 */
export function AgentImportDialog() {
  const open = useStore($agentImportOpen)
  const profile = normalizeProfileKey(useStore($activeGatewayProfile))
  const { t } = useI18n()
  const a = t.agentImport
  const [sources, setSources] = useState<AgentImportSource[] | null>(null)
  const [picked, setPicked] = useState<null | Picked>(null)
  const [chosen, setChosen] = useState<Set<AgentImportGroup>>(new Set())
  const [overwrite, setOverwrite] = useState(false)
  const [step, setStep] = useState<Step>('pick')
  const [report, setReport] = useState<AgentImportReport | null>(null)
  const [error, setError] = useState<null | string>(null)

  useEffect(() => {
    if (!open) {
      return
    }

    setSources(null)
    setPicked(null)
    setChosen(new Set())
    setOverwrite(false)
    setStep('pick')
    setReport(null)
    setError(null)

    let live = true

    detectAgentImports()
      .then(result => live && setSources(result.agents))
      .catch(err => {
        if (live) {
          setSources([])
          setError(err instanceof Error ? err.message : a.failed)
        }
      })

    return () => {
      live = false
    }
  }, [a.failed, open])

  const choose = useCallback((next: Picked) => {
    setPicked(next)
    setChosen(new Set(Object.keys(groupCounts(next.counts)) as AgentImportGroup[]))
  }, [])

  const chooseFolder = useCallback(async () => {
    const paths = await window.factrDesktop?.selectPaths?.({ directories: true, multiple: false, title: a.chooseFolder })
    const path = paths?.[0]

    if (!path) {
      return
    }

    try {
      const preview = await runAgentImport({ agent: 'auto', execute: false, profile, source: path })
      choose({ agent: preview.agent, counts: preview.counts, label: path, path })
    } catch (err) {
      setError(err instanceof Error ? err.message : a.failed)
    }
  }, [a.chooseFolder, a.failed, choose, profile])

  const groups = picked ? groupCounts(picked.counts) : {}
  const total = GROUP_ORDER.reduce((sum, g) => sum + (chosen.has(g) ? (groups[g] ?? 0) : 0), 0)

  const run = async () => {
    if (!picked) {
      return
    }

    setStep('running')
    setError(null)

    try {
      const result = await runAgentImport({
        agent: picked.agent,
        execute: true,
        groups: GROUP_ORDER.filter(g => chosen.has(g)),
        overwrite,
        profile,
        source: picked.path
      })

      setReport(result)
      setStep('result')
      void refreshProfiles().catch(() => undefined)
    } catch (err) {
      setStep('pick')
      setError(err instanceof Error ? err.message : a.failed)
      notifyError(err, a.failed)
    }
  }

  const busy = step === 'running'

  return (
    <Dialog onOpenChange={value => !value && !busy && closeAgentImport()} open={open}>
      <DialogContent className="max-w-lg" data-agent-import="">
        <DialogHeader>
          <DialogTitle>{step === 'result' ? a.resultTitle : a.title}</DialogTitle>
          <DialogDescription>{step === 'result' ? a.target(profile) : a.description}</DialogDescription>
        </DialogHeader>

        {step === 'result' && report ? (
          <Result report={report} />
        ) : (
          <div className="grid gap-4">
            {!picked ? (
              <div className="grid gap-2">
                {sources === null && <p className="text-sm text-(--ui-text-tertiary)">{a.scanning}</p>}
                {(sources ?? []).map(source => {
                  const count = Object.values(groupCounts(source.counts)).reduce((x, y) => x + y, 0)

                  return (
                    <button
                      className={cn(rowClass, 'border-(--ui-stroke-secondary)', source.found && 'hover:bg-(--ui-control-hover-background)')}
                      data-agent={source.agent}
                      disabled={!source.found}
                      key={source.agent}
                      onClick={() => choose({ agent: source.agent, counts: source.counts, label: source.label, path: source.path })}
                      type="button"
                    >
                      <span className="min-w-0">
                        <span className="block text-sm font-medium text-foreground">{source.label}</span>
                        <span className="block truncate text-xs text-(--ui-text-tertiary)">{source.path}</span>
                      </span>
                      <span className="shrink-0 text-xs text-(--ui-text-secondary)">
                        {source.found ? (count > 0 ? a.itemCount(count) : a.nothingToImport) : a.notFound}
                      </span>
                    </button>
                  )
                })}
                <Button className="justify-self-start" onClick={() => void chooseFolder()} type="button" variant="outline">
                  {a.chooseFolder}
                </Button>
              </div>
            ) : (
              <div className="grid gap-3">
                <p className="text-sm text-foreground">
                  {picked.label} <span className="text-(--ui-text-tertiary)">{picked.path}</span>
                </p>
                {GROUP_ORDER.filter(g => groups[g]).length === 0 && (
                  <p className="text-sm text-(--ui-text-tertiary)">{a.nothingToImport}</p>
                )}
                {GROUP_ORDER.filter(g => groups[g]).map(g => (
                  <label className="flex cursor-pointer items-start gap-3" key={g}>
                    <Checkbox
                      checked={chosen.has(g)}
                      className="mt-0.5"
                      onCheckedChange={on =>
                        setChosen(prev => {
                          const next = new Set(prev)

                          if (on) {
                            next.add(g)
                          } else {
                            next.delete(g)
                          }

                          return next
                        })
                      }
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block text-sm text-foreground">
                        {a.groups[g].title} <span className="text-(--ui-text-tertiary)">{groups[g]}</span>
                      </span>
                      <span className="block text-xs text-(--ui-text-tertiary)">{a.groups[g].note}</span>
                    </span>
                  </label>
                ))}
                <label className="flex cursor-pointer items-start gap-3 border-t border-(--ui-stroke-secondary) pt-3">
                  <Checkbox checked={overwrite} className="mt-0.5" onCheckedChange={on => setOverwrite(Boolean(on))} />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm text-foreground">{a.overwriteTitle}</span>
                    <span className="block text-xs text-(--ui-text-tertiary)">{a.overwriteDesc}</span>
                  </span>
                </label>
                <p className="text-xs text-(--ui-text-tertiary)">{a.target(profile)}</p>
              </div>
            )}

            {error && (
              <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                <span>{error}</span>
              </div>
            )}
          </div>
        )}

        <DialogFooter>
          {step === 'result' ? (
            <Button onClick={closeAgentImport} type="button">
              {a.done}
            </Button>
          ) : (
            <>
              {picked ? (
                <Button disabled={busy} onClick={() => setPicked(null)} type="button" variant="ghost">
                  {a.back}
                </Button>
              ) : null}
              {picked && (
                <Button disabled={busy || total === 0} onClick={() => void run()} type="button">
                  {busy ? a.importing : a.importAction(total)}
                </Button>
              )}
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Result({ report }: { report: AgentImportReport }) {
  const { t } = useI18n()
  const a = t.agentImport
  const { conflict, error, imported, skipped } = report.summary
  const paused = report.items.some(item => item.kind === 'cron-job' && item.status === 'imported')

  return (
    <div className="grid gap-3 text-sm" data-agent-import-result="">
      <p className="text-foreground">{a.resultSummary(imported, skipped, conflict, error)}</p>
      {paused && <p className="text-xs text-(--ui-text-tertiary)">{a.pausedJobs}</p>}
      {report.bots && report.bots.length > 0 && (
        <div>
          <p className="text-xs text-(--ui-text-tertiary)">{a.botsCreated}</p>
          <ul className="mt-1 grid gap-0.5">
            {report.bots.map(bot => (
              <li className={cn('text-sm', bot.ok ? 'text-foreground' : 'text-destructive')} key={bot.name}>
                {bot.name}
                {!bot.ok && <span className="text-xs"> {bot.detail}</span>}
              </li>
            ))}
          </ul>
        </div>
      )}
      {report.stripped_secrets && report.stripped_secrets.length > 0 && (
        <div>
          <p className="text-xs text-(--ui-text-tertiary)">{a.strippedSecrets}</p>
          <ul className="mt-1 grid gap-0.5 font-mono text-xs text-(--ui-text-secondary)">
            {report.stripped_secrets.map(name => (
              <li key={name}>{name}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
