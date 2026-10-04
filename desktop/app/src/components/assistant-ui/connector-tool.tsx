import type { ToolCallMessagePartProps } from '@assistant-ui/react'
import type { ConnectionTargetState } from '@factr/shared'
import { type RefObject, useEffect, useRef } from 'react'

import { sessionRoute } from '@/app/routes'
import { ToolFallback } from '@/components/assistant-ui/tool/fallback'
import type { ConnectorRowMark } from '@/components/ui/connector-card'
import type { useI18n } from '@/i18n'
import { recordOf, toolLabels, toolLabelTitle } from '@/lib/connector-tools'
import {
  $connectionRequests,
  connectionOwnerFor,
  type ConnectionRequest,
  type ConnectionTarget
} from '@/store/connection-request'
import { requestGatewayForAgent } from '@/store/gateway'

/** The browser leg of a connection came back through `factr://connections/done`. Show the session
 *  that opened the operation and tell its backend to read the account now instead of at its next
 *  tick. Nothing in the link is trusted to move a row: the op id only names which card to show, and
 *  the backend reads the account itself. An operation this window holds no card for, or one that
 *  already settled, is ignored: the tab can come back long after Continue, and a stale link must
 *  not pull the user away from where they are. */
export async function openConnectionDoneLink(
  op: string,
  navigate: (to: string) => void,
  storedSessionIdFor: (runtimeSessionId: string) => string
): Promise<void> {
  const request = Object.values($connectionRequests.get()).find(entry => entry.opId === op)

  if (!request?.sessionId || request.settled) {
    return
  }

  const storedId = storedSessionIdFor(request.sessionId)
  navigate(sessionRoute(storedId))

  const owner = await connectionOwnerFor(storedId, 'connectors.operation.wake')

  if (!owner) {
    return
  }

  try {
    await requestGatewayForAgent(owner.connectionId, owner.profile, 'connectors.operation.wake', {
      op_id: op,
      owner: { session_id: request.sessionId, type: 'session' }
    })
  } catch {
    // The wake only shortens the wait. The operation can settle and leave the live registry between
    // the link and this RPC (4004); the watcher reads the account at its next tick regardless.
  }
}

/** The card lives on the tool row whose id opened the operation and on no other. */
export function connectionRequestOwnsPart(props: ToolCallMessagePartProps, request: ConnectionRequest | null): boolean {
  return Boolean(request && props.toolCallId === request.toolCallId)
}

interface ConnectorCardPhase {
  mark: ConnectorRowMark
  resolved: boolean
}

export const CONNECTOR_CARD_PHASES = {
  connected: { mark: 'connected', resolved: true },
  expired: { mark: 'idle', resolved: false },
  failed: { mark: 'idle', resolved: false },
  initiated: { mark: 'waiting', resolved: false },
  not_connected: { mark: 'idle', resolved: false },
  pending: { mark: 'idle', resolved: false },
  skipped: { mark: 'idle', resolved: true }
} satisfies Record<ConnectionTargetState, ConnectorCardPhase>

// A disabled verb (a working row, a waiting row with no link yet) refuses focus, and the keyboard
// would land on the document body; so the first control that can take it, else the row itself.
const FOCUSABLE_IN_ROW = 'button:not([disabled]), [href], input:not([disabled])'
// The user is typing a credential; a row moving elsewhere on the card must not take the keyboard.
const EDITABLE = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])'

function focusChangedRow(card: HTMLElement, name: string): void {
  const row = [...card.querySelectorAll<HTMLElement>('[data-connector-row]')].find(
    node => node.dataset.connectorRow === name
  )

  ;(row?.querySelector<HTMLElement>(FOCUSABLE_IN_ROW) ?? row)?.focus()
}

/** Move focus to the row the backend changed. Only while the card already holds focus, and never
 *  out of a field the user is typing in — a transition the user is not looking at must not take
 *  the keyboard away from wherever they are. */
export function useConnectorFocusHandoff(
  targets: readonly ConnectionTarget[],
  cardRef: RefObject<HTMLDivElement | null>
): void {
  const seen = useRef<Map<string, ConnectionTargetState> | null>(null)
  const states = targets.map(target => `${target.name}=${target.state}`).join('|')

  // The ref holds what the last frame said, for comparison only: nothing renders from it, so it
  // cannot lag a render the way a mirrored atom would.
  // eslint-disable-next-line no-restricted-syntax
  useEffect(() => {
    const previous = seen.current
    seen.current = new Map(targets.map(target => [target.name, target.state]))

    const card = cardRef.current

    const moved = targets.find(target => {
      const before = previous?.get(target.name)

      return before !== undefined && before !== target.state
    })

    const active = document.activeElement

    if (!previous || !moved || !card?.contains(active) || active?.matches(EDITABLE)) {
      return
    }

    focusChangedRow(card, moved.name)
    // The target states are the whole input; `states` changes exactly when one of them moves.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [states])
}

type ConnectorCopy = ReturnType<typeof useI18n>['t']['connectors']

export const MARK_LABEL = {
  connected: (copy: ConnectorCopy) => copy.connected,
  idle: (copy: ConnectorCopy) => copy.notConnected,
  waiting: (copy: ConnectorCopy) => copy.waiting
} satisfies Record<ConnectorRowMark, (copy: ConnectorCopy) => string>

const MISSING_CALL_RESULT = { error: 'No result for this call.' }

/** Keep execution output in the standard disclosure, with one row per inner call.
 *  The gateway labels every call the tool_search bridge runs, MCP or local,
 *  so every batch renders the same way. */
export function ConnectorExecution(props: ToolCallMessagePartProps) {
  const labels = toolLabels(props.args)
  const output = recordOf(props.result)
  const results = Array.isArray(output.results) ? output.results : []

  if (labels.length === 0) {
    return <ToolFallback {...props} />
  }

  const input = recordOf(props.args)
  const batch = Array.isArray(input.calls) ? input.calls : [input]

  return (
    <>
      {labels.map((label, index) => {
        // A batch answers one result per call; anything else answers once for the
        // whole call, and every row shows that same outcome (a rejected batch, an error).
        // A batch that answered short says so on the rows it left out.
        const item = results[index] ?? (results.length > 0 ? MISSING_CALL_RESULT : props.result)
        const result = recordOf(item)

        return (
          <ToolFallback
            {...props}
            args={recordOf(recordOf(batch[index]).arguments ?? props.args)}
            isError={Boolean(result.error) || props.isError === true}
            key={`${props.toolCallId}:${index}`}
            result={item}
            toolCallId={`${props.toolCallId}:${index}`}
            toolName={toolLabelTitle(label)}
          />
        )
      })}
    </>
  )
}
