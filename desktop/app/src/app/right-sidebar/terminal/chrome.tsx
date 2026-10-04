import { useStore } from '@nanostores/react'

import { TerminalSlot } from './persistent'
import { TerminalRail } from './rail'
import { $terminals } from './terminals'

/** Pane-side terminal chrome: the body slot (which the persistent overlay chases)
 *  plus the always-on tab rail. Lives in the real pane DOM — NOT the z-4 terminal
 *  overlay — so the rail sits above the collapsed sidebars' z-30 hover-reveal
 *  triggers (z-40, like the thread timeline) and suppresses them while hovered.
 *  The rail switches between terminals, so it shows from the second one on; a
 *  lone terminal closes from its zone tab or the shell's `exit`, and "+" lives
 *  on the zone's header row. Closing the last one hides the pane (reopen
 *  re-creates). */
export function TerminalPaneChrome() {
  const terminals = useStore($terminals)

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <TerminalSlot />
      </div>
      {terminals.length > 1 && <TerminalRail />}
    </div>
  )
}
