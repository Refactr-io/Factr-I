import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

import { StatusbarControls } from '@/app/shell/statusbar-controls'
import { $approvalModes } from '@/store/approval-mode'
import { stubMenuDomApis, stubResizeObserver } from '@/test/jsdom'

import { useApprovalModeStatusbarItem } from './approval-mode-menu'

beforeAll(() => {
  stubResizeObserver()
  stubMenuDomApis()
})

afterEach(() => {
  cleanup()
  $approvalModes.set({})
})

function Harness({
  profile = 'default',
  requestGateway
}: {
  profile?: string
  requestGateway: (method: string, params?: Record<string, unknown>) => Promise<unknown>
}) {
  const item = useApprovalModeStatusbarItem(profile, requestGateway)

  return (
    <MemoryRouter>
      <StatusbarControls items={[item]} />
    </MemoryRouter>
  )
}

describe('approval mode statusbar item', () => {
  it('writes the selected mode through the gateway and updates its shared trigger label', async () => {
    const requestGateway = vi.fn(async (_method, params) => ({ value: params?.value ?? 'manual' }))
    render(<Harness profile="work" requestGateway={requestGateway} />)

    fireEvent.pointerDown(screen.getByRole('button', { name: /manual/i }), { button: 0 })
    fireEvent.click(await screen.findByRole('menuitemradio', { name: /off/i }))

    await waitFor(() => {
      expect(requestGateway).toHaveBeenCalledWith('config.set', { key: 'approvals.mode', value: 'off' })
      expect(screen.getByRole('button', { name: /off/i })).toBeTruthy()
    })
  })

  it('shows Manual by default and never offers Smart', async () => {
    render(<Harness requestGateway={vi.fn(async () => ({}))} />)

    fireEvent.pointerDown(screen.getByRole('button', { name: /manual/i }), { button: 0 })
    expect(await screen.findAllByRole('menuitemradio')).toHaveLength(2)
    expect(screen.queryByRole('menuitemradio', { name: /smart/i })).toBeNull()
  })

  it('shows the stored value for an existing chat and keeps it across a settings-style resync', async () => {
    const { rerender } = render(<Harness requestGateway={vi.fn(async () => ({ value: 'off' }))} />)

    await waitFor(() => expect(screen.getByRole('button', { name: /off/i })).toBeTruthy())

    // Settings open/close: an empty reply must not flip or invent a mode.
    for (const reply of [{}, { value: '' }]) {
      rerender(<Harness requestGateway={vi.fn(async () => reply)} />)
      await Promise.resolve()
      expect(screen.getByRole('button', { name: /off/i })).toBeTruthy()
    }
  })
})
