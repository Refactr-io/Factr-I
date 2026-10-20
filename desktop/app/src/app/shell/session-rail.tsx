import { compactNumber } from '@factr/shared'
import { useStore } from '@nanostores/react'
import { type ReactNode, useEffect, useRef } from 'react'

import { useI18n } from '@/i18n'
import { Folder, GitBranch } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $panesFlipped } from '@/store/layout'
import { projectNameForCwd } from '@/store/projects'
import { $reviewFiles, toggleReview } from '@/store/review'
import { $currentBranch, $currentCwd, $currentUsage } from '@/store/session'
import { $sessionRailOpen } from '@/store/session-rail'

import { TITLEBAR_HEIGHT } from './titlebar'

const SHOWN_FILES = 6

const leaf = (path: string) => path.split(/[\\/]/).filter(Boolean).at(-1) ?? path

function Section({ children, title }: { children: ReactNode; title: string }) {
  return (
    <section className="flex flex-col gap-2 border-b border-(--ui-stroke-secondary) px-3 py-3 last:border-b-0">
      <h3 className="text-xs font-medium text-(--ui-text-tertiary)">{title}</h3>
      {children}
    </section>
  )
}

/**
 * The session rail: what this chat is running in. It rides the chat column's edge under the header and sits
 * over the conversation, like Claude's. It closes on Escape or a click anywhere outside it.
 */
export function SessionRail() {
  const { t } = useI18n()
  const open = useStore($sessionRailOpen)
  const flipped = useStore($panesFlipped)
  const cwd = useStore($currentCwd)
  const branch = useStore($currentBranch)
  const usage = useStore($currentUsage)
  const files = useStore($reviewFiles)
  const ref = useRef<HTMLElement>(null)
  const copy = t.sessionRail

  useEffect(() => {
    if (!open) {
      return
    }

    const close = () => $sessionRailOpen.set(false)

    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        close()
      }
    }

    const onPointer = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null

      // The cluster's own button toggles the rail; every other press outside dismisses it.
      if (ref.current && !ref.current.contains(target) && !target?.closest('[data-tour="pane-session-rail"]')) {
        close()
      }
    }

    window.addEventListener('keydown', onKey)
    window.addEventListener('pointerdown', onPointer, true)

    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('pointerdown', onPointer, true)
    }
  }, [open])

  if (!open) {
    return null
  }

  const project = projectNameForCwd(cwd) ?? (cwd ? leaf(cwd) : null)
  const percent = Math.max(0, Math.min(100, Math.round(usage.context_percent ?? 0)))
  const used = usage.context_used ?? 0
  const max = usage.context_max ?? 0

  return (
    <aside
      aria-label={copy.title}
      className={cn(
        'fixed z-60 flex max-h-[calc(100vh-var(--session-rail-bottom,48px)-0.75rem)] w-[min(20rem,calc(100vw-var(--workspace-left,0px)-var(--workspace-right,0px)-1rem))] flex-col overflow-y-auto rounded-lg border border-(--ui-stroke-secondary) bg-(--ui-bg-elevated) text-[0.8125rem] text-foreground',
        flipped ? 'left-[calc(var(--workspace-left,0px)+0.5rem)]' : 'right-[calc(var(--workspace-right,0px)+0.5rem)]'
      )}
      data-session-rail=""
      ref={ref}
      // below the title-bar band and the 42px pane-icon row at the top of the chat column
      style={{ top: TITLEBAR_HEIGHT + 42 + 6 }}
    >
      <Section title={copy.environment}>
        {cwd ? (
          <>
            <div className="flex items-center gap-2">
              <Folder aria-hidden className="shrink-0 text-(--ui-text-tertiary)" size={15} stroke={1.5} />
              <span className="min-w-0 truncate font-medium">{project}</span>
            </div>
            <p className="min-w-0 truncate pl-[1.375rem] text-xs text-(--ui-text-tertiary)" title={cwd}>
              {cwd}
            </p>
            {branch && (
              <div className="flex items-center gap-2">
                <GitBranch aria-hidden className="shrink-0 text-(--ui-text-tertiary)" size={15} stroke={1.5} />
                <span className="min-w-0 truncate">{branch}</span>
              </div>
            )}
          </>
        ) : (
          <p className="text-xs text-(--ui-text-tertiary)">{copy.noFolder}</p>
        )}
      </Section>

      <Section title={copy.context}>
        <div className="flex items-baseline justify-between gap-2">
          <span>{copy.percentUsed(percent)}</span>
          {max > 0 && (
            <span className="text-xs text-(--ui-text-tertiary)">
              {compactNumber(used)} / {compactNumber(max)}
            </span>
          )}
        </div>
        <div className="h-1.5 overflow-hidden rounded-full bg-(--ui-control-active-background)">
          <div className="h-full rounded-full bg-foreground/70" style={{ width: `${percent}%` }} />
        </div>
      </Section>

      <Section title={copy.changes}>
        {files.length === 0 ? (
          <p className="text-xs text-(--ui-text-tertiary)">{copy.noChanges}</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {files.slice(0, SHOWN_FILES).map(file => (
              <li className="flex items-center justify-between gap-2" key={file.path}>
                <span className="min-w-0 truncate" title={file.path}>
                  {leaf(file.path)}
                </span>
                <span className="shrink-0 text-xs text-(--ui-text-tertiary)">
                  +{file.added} -{file.removed}
                </span>
              </li>
            ))}
          </ul>
        )}
        {files.length > 0 && (
          <button
            className="self-start text-xs text-(--ui-text-secondary) transition-colors duration-100 ease-out hover:text-foreground"
            onClick={() => {
              $sessionRailOpen.set(false)
              toggleReview()
            }}
            type="button"
          >
            {copy.openChanges(files.length)}
          </button>
        )}
      </Section>
    </aside>
  )
}
