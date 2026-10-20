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
import { isFeatureSurfaceOpen, setFeatureSurfaceOpen } from '@/api/client'
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

type Step = 'customize' | 'list' | 'result' | 'running'

interface Picked {
  agent: string
  counts: Record<string, number>
  label: string
  path: string
}

interface Outcome {
  label: string
  report: AgentImportReport
}

const total = (counts: Record<string, number>) => Object.values(groupCounts(counts)).reduce((sum, n) => sum + n, 0)

/**
 * Import another agent's setup into the active profile. It looks for Claude Code, Codex, Hermes, OpenCode and Prime
 * on this computer by itself; one click on Import brings everything over, and "Choose what to import" is there for
 * the rest. Nothing runs until you click.
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
  const [step, setStep] = useState<Step>('list')
  const [working, setWorking] = useState('')
  const [outcomes, setOutcomes] = useState<Outcome[]>([])
  const [error, setError] = useState<null | string>(null)

  useEffect(() => {
    if (!open) {
      return
    }

    // The packaged engine wakes its Python runtime only for requests marked as a feature surface.
    const wasOpen = isFeatureSurfaceOpen()

    setFeatureSurfaceOpen(true)
    setSources(null)
    setPicked(null)
    setChosen(new Set())
    setOverwrite(false)
    setStep('list')
    setOutcomes([])
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
      setFeatureSurfaceOpen(wasOpen)
    }
  }, [a.failed, open])

  const importable = (sources ?? []).filter(source => source.found && total(source.counts) > 0)
  const missing = (sources ?? []).filter(source => !source.found)

  const runOne = useCallback(
    async (target: Picked, groups: AgentImportGroup[], replace: boolean): Promise<Outcome> => {
      const report = await runAgentImport({
        agent: target.agent,
        execute: true,
        groups,
        overwrite: replace,
        profile,
        source: target.path
      })

      return { label: target.label, report }
    },
    [profile]
  )

  const finish = (done: Outcome[]) => {
    setOutcomes(done)
    setStep('result')
    void refreshProfiles().catch(() => undefined)
    window.dispatchEvent(new Event('factr:sessions-changed'))
  }

  const importAll = async (targets: Picked[], groups: AgentImportGroup[] = GROUP_ORDER, replace = false) => {
    setStep('running')
    setError(null)
    const done: Outcome[] = []

    try {
      for (const target of targets) {
        setWorking(target.label)
        done.push(await runOne(target, groups, replace))
      }

      finish(done)
    } catch (err) {
      setStep(picked ? 'customize' : 'list')
      setError(err instanceof Error ? err.message : a.failed)
      notifyError(err, a.failed)

      if (done.length > 0) {
        finish(done)
      }
    }
  }

  const choose = (next: Picked) => {
    setPicked(next)
    setChosen(new Set(Object.keys(groupCounts(next.counts)) as AgentImportGroup[]))
    setStep('customize')
  }

  const chooseFolder = async () => {
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
  }

  const asPicked = (source: AgentImportSource): Picked => ({
    agent: source.agent,
    counts: source.counts,
    label: source.label,
    path: source.path
  })

  const groups = picked ? groupCounts(picked.counts) : {}
  const selectedTotal = GROUP_ORDER.reduce((sum, g) => sum + (chosen.has(g) ? (groups[g] ?? 0) : 0), 0)
  const busy = step === 'running'

  return (
    <Dialog onOpenChange={value => !value && !busy && closeAgentImport()} open={open}>
      <DialogContent className="max-w-lg" data-agent-import="">
        <DialogHeader>
          <DialogTitle>{step === 'result' ? a.resultTitle : a.title}</DialogTitle>
          <DialogDescription>
            {step === 'result' ? a.target(profile) : step === 'customize' ? a.customizeDesc : a.description}
          </DialogDescription>
        </DialogHeader>

        {step === 'result' && <Results outcomes={outcomes} />}

        {step === 'list' && (
          <div className="grid gap-2">
            {sources === null && <p className="text-sm text-(--ui-text-tertiary)">{a.scanning}</p>}
            {sources !== null && importable.length === 0 && (
              <p className="text-sm text-(--ui-text-tertiary)">{a.nothingFound}</p>
            )}
            {importable.map(source => (
              <div
                className="flex items-center justify-between gap-3 rounded-lg border border-(--ui-stroke-secondary) px-3 py-2.5"
                data-agent={source.agent}
                key={source.agent}
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium text-foreground">{source.label}</p>
                  <p className="truncate text-xs text-(--ui-text-tertiary)">{summary(a, source.counts)}</p>
                  <button
                    className="text-xs text-(--ui-text-secondary) underline-offset-2 hover:text-foreground hover:underline"
                    onClick={() => choose(asPicked(source))}
                    type="button"
                  >
                    {a.customize}
                  </button>
                </div>
                <Button onClick={() => void importAll([asPicked(source)])} type="button">
                  {a.importOne}
                </Button>
              </div>
            ))}
            {sources !== null && missing.length > 0 && (
              <p className="pt-1 text-xs text-(--ui-text-tertiary)">
                {a.notFoundList(missing.map(source => source.label).join(', '))}
              </p>
            )}
          </div>
        )}

        {step === 'customize' && picked && (
          <div className="grid gap-3">
            <p className="text-sm text-foreground">
              {picked.label} <span className="text-(--ui-text-tertiary)">{picked.path}</span>
            </p>
            {GROUP_ORDER.filter(g => groups[g]).map(g => (
              <label className="flex cursor-pointer items-start gap-3" key={g}>
                <Checkbox
                  checked={chosen.has(g)}
                  className="mt-0.5 border-(--ui-text-quaternary)"
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
              <Checkbox
                checked={overwrite}
                className="mt-0.5 border-(--ui-text-quaternary)"
                onCheckedChange={on => setOverwrite(Boolean(on))}
              />
              <span className="min-w-0 flex-1">
                <span className="block text-sm text-foreground">{a.overwriteTitle}</span>
                <span className="block text-xs text-(--ui-text-tertiary)">{a.overwriteDesc}</span>
              </span>
            </label>
          </div>
        )}

        {step === 'running' && <p className="text-sm text-(--ui-text-secondary)">{a.importingFrom(working)}</p>}

        {error && (
          <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <DialogFooter>
          {step === 'result' && (
            <>
              <Button onClick={() => setStep('list')} type="button" variant="ghost">
                {a.importAnother}
              </Button>
              <Button onClick={closeAgentImport} type="button">
                {a.done}
              </Button>
            </>
          )}
          {step === 'list' && (
            <>
              <Button onClick={() => void chooseFolder()} type="button" variant="ghost">
                {a.chooseFolder}
              </Button>
              {importable.length > 1 && (
                <Button onClick={() => void importAll(importable.map(asPicked))} type="button">
                  {a.importAll(importable.length)}
                </Button>
              )}
            </>
          )}
          {step === 'customize' && picked && (
            <>
              <Button onClick={() => setStep('list')} type="button" variant="ghost">
                {a.back}
              </Button>
              <Button
                disabled={selectedTotal === 0}
                onClick={() => void importAll([picked], GROUP_ORDER.filter(g => chosen.has(g)), overwrite)}
                type="button"
              >
                {a.importAction(selectedTotal)}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function summary(a: ReturnType<typeof useI18n>['t']['agentImport'], counts: Record<string, number>): string {
  const groups = groupCounts(counts)

  return GROUP_ORDER.filter(g => groups[g])
    .map(g => a.summary[g](groups[g] ?? 0))
    .join(', ')
}

function Results({ outcomes }: { outcomes: Outcome[] }) {
  const { t } = useI18n()
  const a = t.agentImport
  const paused = outcomes.some(o => o.report.items.some(item => item.kind === 'cron-job' && item.status === 'imported'))
  const bots = outcomes.flatMap(o => o.report.bots ?? [])
  const secrets = [...new Set(outcomes.flatMap(o => o.report.stripped_secrets ?? []))]

  return (
    <div className="grid gap-3 text-sm" data-agent-import-result="">
      {outcomes.map(({ label, report }) => (
        <p className="text-foreground" key={label}>
          <span className={cn(outcomes.length > 1 && 'font-medium')}>{outcomes.length > 1 ? `${label}: ` : ''}</span>
          {a.resultSummary(report.summary.imported, report.summary.skipped, report.summary.conflict, report.summary.error)}
        </p>
      ))}
      {paused && <p className="text-xs text-(--ui-text-tertiary)">{a.pausedJobs}</p>}
      {bots.length > 0 && (
        <div>
          <p className="text-xs text-(--ui-text-tertiary)">{a.botsCreated}</p>
          <ul className="mt-1 grid gap-0.5">
            {bots.map(bot => (
              <li className={cn('text-sm', bot.ok ? 'text-foreground' : 'text-destructive')} key={bot.name}>
                {bot.name}
                {!bot.ok && <span className="text-xs"> {bot.detail}</span>}
              </li>
            ))}
          </ul>
        </div>
      )}
      {secrets.length > 0 && (
        <div>
          <p className="text-xs text-(--ui-text-tertiary)">{a.strippedSecrets}</p>
          <ul className="mt-1 grid gap-0.5 font-mono text-xs text-(--ui-text-secondary)">
            {secrets.map(name => (
              <li key={name}>{name}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
