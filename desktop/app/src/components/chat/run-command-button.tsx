import { useStore } from '@nanostores/react'
import { atom } from 'nanostores'
import type { FC, ReactNode } from 'react'

import { Button } from '@/components/ui/button'
import { Tip } from '@/components/ui/tooltip'
import { useI18n } from '@/i18n'
import { Play, Terminal } from '@/lib/icons'
import { cn } from '@/lib/utils'
import { $currentCwd } from '@/store/session'

const SHELL_LANGUAGES = new Set(['bash', 'sh', 'shell', 'zsh'])
const MAX_OUTPUT_CHARS = 100_000

export const isShellLanguage = (language?: string) => SHELL_LANGUAGES.has((language ?? '').trim().toLowerCase())

interface RunState {
  code: null | number
  output: string
  running: boolean
}

// Keyed by command text so a re-mounted (virtualized) block keeps its output.
const $runs = atom<Record<string, RunState>>({})

const patch = (command: string, next: (prev: RunState) => RunState) => {
  const prev = $runs.get()[command] ?? { code: null, output: '', running: false }
  $runs.set({ ...$runs.get(), [command]: next(prev) })
}

/** Runs the command once, only when the user clicks Run; output streams into
 *  the block under the code. Never called automatically. */
export async function runShellCommand(command: string) {
  const api = window.factrDesktop?.terminal

  if (!api?.run || $runs.get()[command]?.running) {
    return
  }

  patch(command, () => ({ code: null, output: '', running: true }))
  const id = await api.run({ command, cwd: $currentCwd.get() || undefined })

  if (!id) {
    patch(command, () => ({ code: null, output: '', running: false }))

    return
  }

  const stopData = api.onData(id, chunk =>
    patch(command, prev => ({ ...prev, output: (prev.output + chunk).slice(-MAX_OUTPUT_CHARS) }))
  )

  const stopExit = api.onExit(id, exit => {
    stopData()
    stopExit()
    patch(command, prev => ({ ...prev, code: exit.code ?? null, running: false }))
  })
}

export const RunCommandButton: FC<{ className?: string; command: string }> = ({ className, command }) => {
  const { t } = useI18n()
  const running = Boolean(useStore($runs)[command]?.running)

  return (
    <Tip label={t.assistant.tool.runInTerminal}>
      <Button
        aria-label={t.assistant.tool.runInTerminal}
        className={cn('h-5 rounded-md px-1', className)}
        disabled={running}
        onClick={() => void runShellCommand(command)}
        size="icon"
        type="button"
        variant="ghost"
      >
        <Play className="size-3" />
      </Button>
    </Tip>
  )
}

/** Output of a Run click: a small block under the code with a ">_" mark. */
export const RunOutput: FC<{ command: string }> = ({ command }) => {
  const run = useStore($runs)[command]

  if (!run || (!run.running && !run.output && run.code === null)) {
    return null
  }

  return (
    <div className="relative mt-1.5 rounded-lg bg-muted/40 px-3 py-2.5" data-slot="run-output">
      <Terminal aria-hidden className="absolute right-4 top-2 size-3 text-muted-foreground" />
      <pre className="m-0 max-h-72 overflow-auto whitespace-pre-wrap break-all pr-6 font-mono text-xs">
        {run.output || (run.running ? '…' : '')}
      </pre>
      {!run.running && run.code ? <div className="mt-1 text-xs text-muted-foreground">exit {run.code}</div> : null}
    </div>
  )
}

export const ShellRun: FC<{ children: ReactNode; command: string }> = ({ children, command }) => (
  <>
    {children}
    <RunOutput command={command} />
  </>
)
