import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { detectAgentImports, runAgentImport } from '@/api/agent-import'
import { $agentImportOpen, openAgentImport } from '@/store/agent-import'

import { AgentImportDialog } from './agent-import-dialog'

vi.mock('@/api/agent-import', async importOriginal => ({
  ...(await importOriginal<typeof import('@/api/agent-import')>()),
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

beforeEach(() => {
  vi.mocked(detectAgentImports).mockResolvedValue({
    agents: [
      { agent: 'claude-code', counts: { bot: 2, chats: 5, skill: 4 }, found: true, label: 'Claude Code', path: '/h/.claude' },
      { agent: 'hermes', counts: {}, found: false, label: 'Hermes', path: '/h/.hermes' }
    ],
    groups: ['skills', 'mcp', 'memory', 'prompt', 'cron', 'bots', 'chats', 'permissions']
  })
  openAgentImport()
})

afterEach(() => {
  cleanup()
  $agentImportOpen.set(false)
  vi.clearAllMocks()
})

it('lists what was found and disables agents that are not on this computer', async () => {
  render(<AgentImportDialog />)

  const claude = (await screen.findByRole('button', { name: /Claude Code/ })) as HTMLButtonElement
  expect(claude.disabled).toBe(false)
  expect(claude.textContent).toContain('11 items')
  expect((screen.getByRole('button', { name: /Hermes/ }) as HTMLButtonElement).disabled).toBe(true)
})

it('imports only the groups that stay ticked, into the active profile, and shows the result', async () => {
  vi.mocked(runAgentImport).mockResolvedValueOnce(
    report({
      bots: [{ detail: '/p/scout', name: 'scout', ok: true }],
      stripped_secrets: ['mcp_servers.fs.env.API_KEY'],
      items: [{ kind: 'cron-job', reason: '', source: 'j', status: 'imported' }]
    })
  )
  render(<AgentImportDialog />)

  fireEvent.click(await screen.findByRole('button', { name: /Claude Code/ }))
  expect(screen.queryByText('Skills')).not.toBeNull()
  fireEvent.click(screen.getAllByRole('checkbox')[2]) // chats off (skills, bots, chats)
  fireEvent.click(screen.getByRole('button', { name: /Import 6 items/ }))

  await waitFor(() =>
    expect(runAgentImport).toHaveBeenCalledWith({
      agent: 'claude-code',
      execute: true,
      groups: ['skills', 'bots'],
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

it('keeps the dialog on the pick step and says why when the import fails', async () => {
  vi.mocked(runAgentImport).mockRejectedValueOnce(new Error('boom'))
  render(<AgentImportDialog />)

  fireEvent.click(await screen.findByRole('button', { name: /Claude Code/ }))
  fireEvent.click(screen.getByRole('button', { name: /Import 11 items/ }))

  expect(await screen.findByText('boom')).not.toBeNull()
  expect((screen.getByRole('button', { name: /Import 11 items/ }) as HTMLButtonElement).disabled).toBe(false)
})
