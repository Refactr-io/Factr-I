import { atom } from 'nanostores'

// Sessions opened only to be read (a cron run's output): the transcript shows,
// the composer does not take input. In module memory on purpose: a reopen from
// Run history marks it again, and nothing else should ever make a chat read-only.
export const $readOnlySessionIds = atom<ReadonlySet<string>>(new Set())

/** Titles for sessions that are not in the sidebar list (a cron run opened from Run history). */
export const $sessionTitleOverrides = atom<Readonly<Record<string, string>>>({})

export function markSessionReadOnly(id: string, title?: string): void {
  if (!$readOnlySessionIds.get().has(id)) {
    $readOnlySessionIds.set(new Set([...$readOnlySessionIds.get(), id]))
  }

  if (title && $sessionTitleOverrides.get()[id] !== title) {
    $sessionTitleOverrides.set({ ...$sessionTitleOverrides.get(), [id]: title })
  }
}

/** The injected scheduler preamble opens every cron run's prompt. */
export const CRON_PREAMBLE_RE = /^\s*\[IMPORTANT:\s*You are running as a scheduled cron job/i

/** Run title for the history list: the job and the time, never the injected preamble. */
export function cronRunTitle(
  run: { last_active?: null | number; preview?: null | string; started_at?: null | number; title?: null | string },
  jobName: string,
  formatTime: (seconds?: null | number) => string
): string {
  const title = run.title?.trim()

  if (title && !CRON_PREAMBLE_RE.test(title)) {
    return title
  }

  return `${jobName} · ${formatTime(run.last_active || run.started_at)}`
}

/** What the engine says about a session that is not in the sidebar list (a cron run): its title,
 *  source and working directory from the resume response. Lets the title bar and sidebar name it. */
export interface ExternalSessionMeta {
  cwd?: null | string
  openedAt: number
  source?: null | string
  title: string
}

export const $externalSessions = atom<Readonly<Record<string, ExternalSessionMeta>>>({})

export function rememberExternalSession(
  id: string,
  meta: { cwd?: null | string; source?: null | string; title?: null | string }
): void {
  const title = meta.title?.trim()

  if (!id || !title) {
    return
  }

  const known = $externalSessions.get()[id]

  if (known?.title === title && known.source === meta.source && known.cwd === meta.cwd) {
    return
  }

  $externalSessions.set({
    ...$externalSessions.get(),
    [id]: { cwd: meta.cwd, openedAt: known?.openedAt ?? Date.now() / 1000, source: meta.source, title }
  })

  if ($sessionTitleOverrides.get()[id] !== title) {
    $sessionTitleOverrides.set({ ...$sessionTitleOverrides.get(), [id]: title })
  }

  // The engine names a cron run's source: that alone makes it a read-only transcript, however it was opened
  // (Run history marks it too, but a deep link or a restored tab never passes through there).
  if (meta.source === 'cron' && !$readOnlySessionIds.get().has(id)) {
    $readOnlySessionIds.set(new Set([...$readOnlySessionIds.get(), id]))
  }
}
