import { type OrbState, ThinkingOrb } from 'thinking-orbs'

import { usePaneVisible } from '@/components/pane-shell/pane-visibility'
import { isFileEditTool } from '@/lib/tool-render-class'
import { cn } from '@/lib/utils'

interface AgentOrbProps {
  /** Omit when a neighbouring label already names the state. */
  ariaLabel?: string
  className?: string
  /** Freeze on the current frame (a surface fading out keeps a still orb). */
  paused?: boolean
  /** The two tuned scales: 20 inline with text, 64 on its own (a pane that
   *  is loading). */
  size?: 20 | 64
  state?: OrbState
}

/**
 * The "the agent is working" mark: a `ThinkingOrb` (20px inline, 64px on its own) in a
 * cell that is a `role="status"` when given a label (the canvas itself is
 * decorative). The orb follows the `dark` class on `<html>` by itself and
 * holds a still frame under `prefers-reduced-motion`; it also freezes while
 * its pane is a hidden kept-alive tab so unseen orbs don't burn frames.
 */
export function AgentOrb({ ariaLabel, className, paused = false, size = 20, state = 'working' }: AgentOrbProps) {
  const visible = usePaneVisible()

  return (
    <span
      aria-hidden={ariaLabel ? undefined : true}
      aria-label={ariaLabel}
      className={cn('inline-flex shrink-0 items-center justify-center', size === 20 ? 'size-5' : 'size-16', className)}
      data-paused={paused || !visible ? 'true' : undefined}
      data-slot="agent-orb"
      role={ariaLabel ? 'status' : undefined}
    >
      <ThinkingOrb aria-hidden="true" paused={paused || !visible} size={size} state={state} />
    </span>
  )
}

// Tools that go looking for something rather than doing it.
const SEARCH_TOOLS = new Set(['web_search', 'web_extract', 'search_files', 'session_search', 'browser_navigate'])

/** Which orb a running tool wears: search looks, writes compose, memory and
 *  skills reshape what the agent knows, sub-agents weave; the rest work. */
export function orbStateForTool(toolName: string | undefined): OrbState {
  if (!toolName) {
    return 'working'
  }

  if (SEARCH_TOOLS.has(toolName)) {
    return 'searching'
  }

  if (isFileEditTool(toolName)) {
    return 'composing'
  }

  if (toolName === 'memory' || toolName === 'skill_manage') {
    return 'shaping'
  }

  return toolName === 'delegate_task' ? 'weaving' : 'working'
}
