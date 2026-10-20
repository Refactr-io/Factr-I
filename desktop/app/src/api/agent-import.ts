import { factrApi } from './client'

export type AgentImportGroup = 'bots' | 'chats' | 'cron' | 'mcp' | 'memory' | 'permissions' | 'prompt' | 'skills'

export interface AgentImportSource {
  agent: string
  /** Items a preview would bring in, per backend item kind (see `groupCounts`). */
  counts: Record<string, number>
  found: boolean
  label: string
  path: string
}

export interface AgentImportReport {
  agent: string
  bots?: Array<{ detail: string; name: string; ok: boolean }>
  counts: Record<string, number>
  dry_run: boolean
  items: Array<{ details?: unknown; kind: string; reason: string; source: null | string; status: string }>
  stripped_secrets?: string[]
  summary: { conflict: number; error: number; imported: number; skipped: number }
}

export const AGENT_IMPORT_TIMEOUT_MS = 10 * 60_000

export function detectAgentImports(): Promise<{ agents: AgentImportSource[]; groups: AgentImportGroup[] }> {
  return factrApi({ path: '/api/agent-import/detect', timeoutMs: 60_000 })
}

export function runAgentImport(body: {
  agent: string
  execute: boolean
  groups?: AgentImportGroup[]
  overwrite?: boolean
  profile?: string
  source?: string
}): Promise<AgentImportReport & { ok: boolean }> {
  return factrApi({ path: '/api/agent-import', method: 'POST', body, timeoutMs: AGENT_IMPORT_TIMEOUT_MS })
}

/** Backend item kinds → the checkbox groups the dialog offers. */
const KIND_GROUP: Record<string, AgentImportGroup> = {
  'agents-md': 'memory',
  bot: 'bots',
  chats: 'chats',
  'claude-md': 'memory',
  'command-allowlist': 'permissions',
  'command-denylist': 'permissions',
  'cron-job': 'cron',
  'mcp-servers': 'mcp',
  memories: 'memory',
  skill: 'skills',
  'system-prompt': 'prompt'
}

export const GROUP_ORDER: AgentImportGroup[] = ['skills', 'mcp', 'memory', 'prompt', 'cron', 'bots', 'chats', 'permissions']

export function groupCounts(counts: Record<string, number>): Partial<Record<AgentImportGroup, number>> {
  const out: Partial<Record<AgentImportGroup, number>> = {}

  for (const [kind, n] of Object.entries(counts)) {
    const group = KIND_GROUP[kind]

    if (group && n > 0) {
      out[group] = (out[group] ?? 0) + n
    }
  }

  return out
}
