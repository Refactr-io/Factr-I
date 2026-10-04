import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'

import { RunHistory } from './run-history'

const request = vi.fn()
const stop = vi.fn()
vi.mock('@/api/client', () => ({
  factrApi: (...args: unknown[]) => request(...args),
  profileScoped: () => ({}),
  getApiRequestProfile: () => 'default'
}))
vi.mock('@/store/gateway', () => ({ requestGatewayForProfile: (...args: unknown[]) => stop(...args) }))

afterEach(() => {
  request.mockReset()
  stop.mockReset()
})

it('shows a persisted run, its timeline, and stops the selected session', async () => {
  request.mockImplementation(({ path }: { path: string }) =>
    path.includes('/run?')
      ? Promise.resolve({
          spans: [
            {
              id: 'span',
              kind: 'execute_tool',
              name: 'repl',
              status: 'complete',
              started_at_ms: 1000,
              ended_at_ms: 2000,
              input_tokens: 0,
              output_tokens: 0,
              cost_usd: 0,
              error: null,
              input: null,
              output: null
            }
          ],
          content: null
        })
      : Promise.resolve({
          capture_content: false,
          dropped_events: 0,
          runs: [
            {
              id: 'run',
              session_id: 'session',
              parent_id: null,
              root_id: 'run',
              kind: 'cron',
              title: 'Accounting cron',
              model: 'local',
              provider: 'Ollama',
              status: 'running',
              started_at_ms: 1000,
              ended_at_ms: null,
              input_tokens: 2,
              output_tokens: 3,
              cache_read_tokens: 0,
              cache_write_tokens: 0,
              cost_usd: 0,
              unpriced_calls: 1,
              error: null
            }
          ]
        })
  )
  stop.mockResolvedValue({ interrupted: true })
  render(<RunHistory />)
  expect(await screen.findByText('repl')).toBeTruthy()
  expect(screen.getByText('5')).toBeTruthy()
  expect(screen.getAllByText(/Accounting cron/).length).toBeGreaterThan(0)
  expect(screen.getAllByText(/cron/).length).toBeGreaterThan(0)
  expect(screen.getAllByText(/unpriced/).length).toBeGreaterThan(0)
  fireEvent.click(screen.getByRole('button', { name: 'Stop run' }))
  await waitFor(() => expect(stop).toHaveBeenCalledWith('default', 'session.interrupt', { session_id: 'session' }))
})

it('marks a run open for over an hour as wedged and lists sessions, alerts and memory deletions', async () => {
  const run = {
    id: 'old',
    session_id: 'session',
    parent_id: null,
    root_id: 'old',
    kind: 'invoke_agent',
    title: 'Stuck turn',
    model: 'local',
    provider: 'Ollama',
    status: 'running',
    started_at_ms: 1000,
    ended_at_ms: null,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_write_tokens: 0,
    cost_usd: null,
    unpriced_calls: 0,
    error: null,
    outcome: 'wedged',
    step_count: 0,
    tools_called: 0
  }
  request.mockImplementation(({ path }: { path: string }) =>
    path.includes('/run?')
      ? Promise.resolve({ spans: [], content: null })
      : path.includes('/sessions')
        ? Promise.resolve({
            sessions: [
              {
                session_id: 'session',
                turns: 2,
                subagents: 0,
                failed: 0,
                cancelled: 0,
                wedged: 1,
                running: 1,
                input_tokens: 1,
                output_tokens: 1,
                cost_usd: null,
                model: 'local',
                provider: 'Ollama',
                trigger: 'desktop',
                last_outcome: 'wedged',
                last_started_at_ms: 1000
              }
            ]
          })
        : path.includes('/alerts')
          ? Promise.resolve({
              monitors: [
                {
                  id: 'wedged',
                  state: 'firing',
                  severity: 'page',
                  title: 'wedged',
                  detail: '1 run never finished',
                  since_ms: 1000
                }
              ]
            })
          : path.includes('/memory-audit')
            ? Promise.resolve({
                deletions: [
                  {
                    id: 1,
                    deleted_at_ms: 1000,
                    memory_id: 'm1',
                    scope: 'global',
                    category: 'preference',
                    length: 14,
                    actor: null,
                    actor_via: 'unidentified'
                  }
                ]
              })
            : Promise.resolve({ capture_content: false, dropped_events: 0, runs: [run] })
  )
  render(<RunHistory />)
  expect((await screen.findAllByText('wedged')).length).toBeGreaterThan(0)
  expect(await screen.findByRole('alert')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }))
  expect(await screen.findByText(/2 turns/)).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Alerts' }))
  expect(await screen.findByText('1 run never finished')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: 'Memory' }))
  expect(await screen.findByText(/preference · 14 chars/)).toBeTruthy()
})
