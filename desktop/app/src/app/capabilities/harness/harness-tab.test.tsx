// @vitest-environment jsdom
import { QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'

import { en } from '@/i18n/en'
import { queryClient } from '@/lib/query-client'

const getHarness = vi.hoisted(() => vi.fn())
const rollbackHarness = vi.hoisted(() => vi.fn())
const requestGateway = vi.hoisted(() => vi.fn(async () => ({})))
const session = vi.hoisted(() => ({ id: 's1' as null | string }))

vi.mock('@/factr', () => ({ getHarness, rollbackHarness }))
vi.mock('@/app/gateway/hooks/use-gateway-request', () => ({ useGatewayRequest: () => ({ requestGateway }) }))
vi.mock('@/store/notifications', () => ({ notifyError: vi.fn() }))
vi.mock('@nanostores/react', () => ({ useStore: () => session.id }))
vi.mock('@/store/session', () => ({ $activeSessionId: {} }))

import { HarnessTab } from './harness-tab'

const h = en.skills.harness

const data = {
  entries: [
    {
      id: 'e1',
      title: 'Scaffold order',
      content: 'Cargo first',
      path: 'c/s',
      scope: 'global',
      source: 'refine',
      timestamp: 1
    }
  ],
  changesets: [
    {
      id: 'c1',
      summary: 'learned scaffold',
      rationale: '',
      edits: 1,
      rolledBack: false,
      isRollback: false,
      timestamp: 1
    },
    { id: 'c0', summary: 'old', rationale: '', edits: 2, rolledBack: true, isRollback: false, timestamp: 1 }
  ]
}

const view = () =>
  render(
    <QueryClientProvider client={queryClient}>
      <HarnessTab />
    </QueryClientProvider>
  )

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.clearAllMocks()
  session.id = 's1'
})

test('lists learned instructions and rolls a changeset back', async () => {
  getHarness.mockResolvedValue(data)
  rollbackHarness.mockResolvedValue({ ok: true, message: 'x' })
  view()

  expect(await screen.findByText('Cargo first')).toBeTruthy()
  // only the live changeset offers Rollback; the rolled-back one says so
  expect(screen.getAllByRole('button', { name: h.rollback })).toHaveLength(1)
  expect(screen.getByText(h.rolledBack)).toBeTruthy()

  fireEvent.click(screen.getByRole('button', { name: h.rollback }))
  await waitFor(() => expect(rollbackHarness).toHaveBeenCalledWith('c1'))
})

test('Refine now runs /refine on the open chat, and needs one', async () => {
  getHarness.mockResolvedValue(data)
  view()
  fireEvent.click(await screen.findByRole('button', { name: h.refineNow }))
  await waitFor(() =>
    expect(requestGateway).toHaveBeenCalledWith('slash.exec', { command: 'refine', session_id: 's1' })
  )

  cleanup()
  session.id = null
  view()
  expect(((await screen.findByRole('button', { name: h.refineNow })) as HTMLButtonElement).disabled).toBe(true)
})
