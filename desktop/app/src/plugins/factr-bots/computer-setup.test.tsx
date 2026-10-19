import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { ComputerGate } from './screen-pane'

afterEach(() => { cleanup(); delete (window as any).factrDesktop })
const computer = { state: 'no-docker' as const, status: { state: 'no-docker' as const, reason: 'not-installed' as const, busy: false }, start: async () => {}, stop: async () => {} }
describe('computer setup gate', () => {
  it('uses the consent-owned main-process setup and starts only after ready', async () => {
    const setup = vi.fn(async () => ({ state: 'ready' }))
    ;(window as any).factrDesktop = { botComputer: { setup } }
    const start = vi.fn()
    render(<ComputerGate computer={computer} onStart={start} />)
    fireEvent.click(screen.getByRole('button'))
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1))
    expect(setup).toHaveBeenCalledTimes(1)
  })
  it('shows a setup failure, preserves the retry button and does not start the bot', async () => {
    ;(window as any).factrDesktop = { botComputer: { setup: async () => ({ state: 'error', message: 'Installer cancelled. Try again.' }) } }
    const start = vi.fn()
    render(<ComputerGate computer={computer} onStart={start} />)
    fireEvent.click(screen.getByRole('button'))
    await screen.findByText('Installer cancelled. Try again.')
    expect((screen.getByRole('button') as HTMLButtonElement).disabled).toBe(false)
    expect(start).not.toHaveBeenCalled()
  })
})
