import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter, Route, Routes, useLocation } from 'react-router'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type * as FactrApi from '@/factr'

import { LegacyAgentsRedirect } from '../contrib/surfaces'

import { CommandCenterView } from './index'

vi.mock('../agents', () => ({ RunsActivity: ({ onOpenSession }: { onOpenSession: (id: string) => void }) => <button onClick={() => onOpenSession('session-1')}>Open recorded session</button> }))
vi.mock('./maintenance', () => ({ MaintenancePanel: () => <div>Maintenance actions</div> }))
vi.mock('@/factr', async importOriginal => ({ ...(await importOriginal<typeof FactrApi>()), getStatus: vi.fn(async () => ({ gateway_running: true })), getLogs: vi.fn(async () => ({ lines: [] })), getUsageAnalytics: vi.fn(async () => ({ totals: {}, daily: [], by_model: [] })), getActionStatus: vi.fn(), restartGateway: vi.fn(), updateFactr: vi.fn() }))

afterEach(cleanup)

function Where() { const { search } = useLocation();

 return <output>{search}</output> }

function mount(path = '/command-center') {
  const onOpenSession = vi.fn()
  render(<MemoryRouter initialEntries={[path]}><CommandCenterView onClose={vi.fn()} onDeleteSession={vi.fn()} onOpenSession={onOpenSession} /><Where /></MemoryRouter>)

  return onOpenSession
}

describe('Activity', () => {
  it('redirects the old agents route to Runs', async () => {
    render(<MemoryRouter initialEntries={['/agents']}><Routes><Route element={<LegacyAgentsRedirect />} path="agents" /><Route element={<Where />} path="command-center" /></Routes></MemoryRouter>)
    expect(await screen.findByText('?section=runs')).toBeTruthy()
  })
  it('defaults to Runs and preserves jump to session', () => {
    const open = mount()
    expect(screen.getByRole('radio', { name: 'Runs' }).getAttribute('aria-checked')).toBe('true')
    expect(screen.queryByRole('radio', { name: 'Scheduled' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Open recorded session' }))
    expect(open).toHaveBeenCalledWith('session-1')
  })
  it('flattens maintenance and sessions into System', async () => {
    mount()
    fireEvent.click(screen.getByRole('radio', { name: 'System' }))
    expect(await screen.findByText('Maintenance actions')).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Sessions' })).toBeTruthy()
    expect(screen.getByText('?section=system')).toBeTruthy()
  })
  it('retains Usage analytics', async () => {
    mount('/command-center?section=usage')
    expect(screen.getByRole('radio', { name: 'Usage' }).getAttribute('aria-checked')).toBe('true')
    expect(await screen.findByText('Daily tokens')).toBeTruthy()
  })
})
