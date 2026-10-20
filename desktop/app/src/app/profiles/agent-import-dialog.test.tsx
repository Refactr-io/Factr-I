import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type * as AgentImportApi from '@/api/agent-import'
import { detectAgentImports, runAgentImport } from '@/api/agent-import'
import { $agentImportOpen, openAgentImport } from '@/store/agent-import'

import { AgentImportDialog } from './agent-import-dialog'

vi.mock('@/api/agent-import', async importOriginal => ({
  ...(await importOriginal<typeof AgentImportApi>()),
  detectAgentImports: vi.fn(),
  runAgentImport: vi.fn()
}))

vi.mock('@/store/profile', () => ({
  $activeGatewayProfile: { get: () => 'default', listen: () => () => undefined, subscribe: () => () => undefined },
  normalizeProfileKey: (name: string) => name || 'default',
  refreshProfiles: vi.fn(async () => [])
}))

const report = (overrides = {}) => ({
  agent: 'hermes',
  counts: {},
  dry_run: false,
  items: [],
  ok: true,
  summary: { conflict: 0, error: 0, imported: 3, skipped: 1 },
  ...overrides
})

const ALL = ['skills', 'mcp', 'memory', 'prompt', 'cron', 'bots', 'chats', 'permissions']

beforeEach(() => {
  vi.mocked(detectAgentImports).mockResolvedValue({
    agents: [
      { agent: 'claude-code', counts: { bot: 2, chats: 5, skill: 4 }, found: true, label: 'Claude Code', path: '/h/.claude' },
      { agent: 'hermes', counts: { 'cron-job': 2, skill: 3 }, found: true, label: 'Hermes', path: '/h/.hermes' },
      { agent: 'codex', counts: {}, found: false, label: 'Codex', path: '/h/.codex' },
      { agent: 'prime', counts: {}, found: false, label: 'Prime', path: '/h/.prime' }
    ],
    groups: ALL as never
  })
  openAgentImport()
})

afterEach(() => {
  cleanup()
  $agentImportOpen.set(false)
  vi.clearAllMocks()
})

it('finds agents by itself, summarises what each has, and names the ones it could not find', async () => {
  render(<AgentImportDialog />)

  await screen.findByText('Claude Code')
  expect(screen.queryByText('4 skills, 2 bots, 5 chats')).not.toBeNull()
  expect(screen.queryByText('3 skills, 2 scheduled jobs')).not.toBeNull()
  expect(screen.queryByText('Not found on this computer: Codex, Prime')).not.toBeNull()
  expect(runAgentImport).not.toHaveBeenCalled() // nothing happens until you click
})

it('one click on Import brings everything over from that agent, into the active profile', async () => {
  vi.mocked(runAgentImport).mockResolvedValueOnce(
    report({
      bots: [{ detail: '/p/scout', name: 'scout', ok: true }],
      items: [{ kind: 'cron-job', reason: '', source: 'j', status: 'imported' }],
      stripped_secrets: ['mcp_servers.fs.env.API_KEY']
    })
  )
  render(<AgentImportDialog />)

  const row = (await screen.findByText('Claude Code')).closest('[data-agent]') as HTMLElement
  fireEvent.click(row.querySelector('button:not([class*="underline"])') as HTMLButtonElement)

  await waitFor(() =>
    expect(runAgentImport).toHaveBeenCalledWith({
      agent: 'claude-code',
      execute: true,
      groups: ALL,
      overwrite: false,
      profile: 'default',
      source: '/h/.claude'
    })
  )
  expect(await screen.findByText('3 imported, 1 skipped, 0 already existed')).not.toBeNull()
  expect(screen.queryByText('scout')).not.toBeNull()
  expect(screen.queryByText('mcp_servers.fs.env.API_KEY')).not.toBeNull()
  expect(screen.queryByText(/paused until you resume/i)).not.toBeNull()
})

it('Import all runs every detected agent in turn and reports each', async () => {
  vi.mocked(runAgentImport).mockResolvedValueOnce(report()).mockResolvedValueOnce(report({ summary: { conflict: 1, error: 0, imported: 2, skipped: 0 } }))
  render(<AgentImportDialog />)

  fireEvent.click(await screen.findByRole('button', { name: 'Import all (2)' }))

  await waitFor(() => expect(runAgentImport).toHaveBeenCalledTimes(2))
  expect(vi.mocked(runAgentImport).mock.calls.map(call => call[0].agent)).toEqual(['claude-code', 'hermes'])
  expect(await screen.findByText('2 imported, 0 skipped, 1 already existed')).not.toBeNull()
  expect(screen.queryByText(/Claude Code:/)).not.toBeNull()
})

it('"Choose what to import" runs only the ticked groups, and replaces only when asked', async () => {
  vi.mocked(runAgentImport).mockResolvedValueOnce(report())
  render(<AgentImportDialog />)

  const row = (await screen.findByText('Claude Code')).closest('[data-agent]') as HTMLElement
  fireEvent.click(row.querySelector('button[class*="underline"]') as HTMLButtonElement)
  fireEvent.click(screen.getAllByRole('checkbox')[2]) // chats off (skills, bots, chats)
  fireEvent.click(screen.getByRole('button', { name: /Import 6 items/ }))

  await waitFor(() =>
    expect(runAgentImport).toHaveBeenCalledWith(expect.objectContaining({ agent: 'claude-code', groups: ['skills', 'bots'], overwrite: false }))
  )
})

it('keeps the list and says why when the import fails', async () => {
  vi.mocked(runAgentImport).mockRejectedValueOnce(new Error('boom'))
  render(<AgentImportDialog />)

  const row = (await screen.findByText('Claude Code')).closest('[data-agent]') as HTMLElement
  fireEvent.click(row.querySelector('button:not([class*="underline"])') as HTMLButtonElement)

  expect(await screen.findByText('boom')).not.toBeNull()
  expect(screen.queryByText('Claude Code')).not.toBeNull()
})
