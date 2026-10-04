import type { RpcMethods } from '@factr/shared'

import type { ProfileScope } from '@/factr'
import { requestGatewayForAgent } from '@/store/gateway'

const MCP_TIMEOUT_MS = 45_000

interface GatewayRoute {
  connectionId: null | string
  profile: string
}

function routeOf(scope: ProfileScope): GatewayRoute {
  if (scope instanceof Object) {
    return {
      connectionId: (scope.connectionId ?? '').trim() || null,
      profile: (scope.profile ?? '').trim()
    }
  }

  return { connectionId: null, profile: (scope ?? '').trim() }
}

type Urgency = 'background' | 'foreground'

type GatewayParams = NonNullable<Parameters<typeof requestGatewayForAgent>[3]>

function call<M extends keyof RpcMethods>(
  scope: ProfileScope,
  method: M,
  params: RpcMethods[M]['params'],
  urgency: Urgency = 'background'
): Promise<RpcMethods[M]['result']> {
  const { connectionId, profile } = routeOf(scope)

  // SAFETY: every `RpcMethods[M]['params']` is a generated object type, which is the record the router takes.
  const payload = params as RpcMethods[M]['params'] & GatewayParams

  return requestGatewayForAgent<RpcMethods[M]['result']>(
    connectionId,
    profile,
    method,
    payload,
    MCP_TIMEOUT_MS,
    undefined,
    { spawnPriority: urgency }
  )
}

export const setMcpBearerToken = (scope: ProfileScope, name: string, value: string) =>
  call(scope, 'mcp.servers.set_api_key', { name, value }, 'foreground')

export const listMcpServers = (scope: ProfileScope) => call(scope, 'mcp.servers.list', {})

export const mcpServerStatus = (scope: ProfileScope) => call(scope, 'mcp.servers.status', {})
