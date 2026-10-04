import type { ToolCallMessagePartProps } from '@assistant-ui/react'
import { describe, expect, it, vi } from 'vitest'

import { connectionRequestOwnsPart } from '@/components/assistant-ui/connector-tool'
import type { ConnectionRequest } from '@/store/connection-request'

const REQUEST: ConnectionRequest = {
  deadlineAt: 1_800_000_000,
  opId: 'operation-1',
  seq: 0,
  toolCallId: 'connector-call-1',
  sessionId: 'session-1',
  settled: false,
  settledBy: null,
  targets: []
}

function props(): ToolCallMessagePartProps {
  const args = { action: 'authorize', connectors: [{ mcp: true, name: 'linear' }] }

  return {
    addResult: vi.fn(),
    args,
    argsText: JSON.stringify(args),
    isError: false,
    respondToApproval: vi.fn(),
    result: undefined,
    resume: vi.fn(),
    status: { type: 'running' },
    toolCallId: 'connector-call-1',
    toolName: 'manage_connections',
    type: 'tool-call'
  }
}

describe('connection request ownership', () => {
  it('never binds to a tool row from a different call, even for the same apps', () => {
    // A second authorize for the same server opens a new operation on a new tool_call_id. The old row must stay
    // dead: it is matched by id only, never by server names.
    expect(
      connectionRequestOwnsPart(props(), { ...REQUEST, opId: 'operation-2', toolCallId: 'connector-call-2' })
    ).toBe(false)
    expect(connectionRequestOwnsPart(props(), REQUEST)).toBe(true)
  })
})
