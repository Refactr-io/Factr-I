// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter, useLocation } from 'react-router'
import { afterEach, describe, expect, it } from 'vitest'

import { SidebarProvider } from '@/components/ui/sidebar'

import { AGENTS_ROUTE } from '../../routes'

import { SIDEBAR_NAV } from './nav-model'
import { SidebarRail } from './rail'

function Where() {
  return <output data-testid="where">{useLocation().pathname}</output>
}

const renderRail = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <SidebarProvider>
        <SidebarRail />
        <Where />
      </SidebarProvider>
    </MemoryRouter>
  )

afterEach(cleanup)

describe('Activity rail entry', () => {
  it('removes the separate run-history row while keeping the route alias', () => {
    expect(SIDEBAR_NAV.some(entry => entry.id === 'agents')).toBe(false)
    expect(AGENTS_ROUTE).toBe('/agents')
  })

  it('opens Activity in the former Command Center slot', () => {
    renderRail('/')
    expect(screen.queryByRole('button', { name: 'Run history' })).toBeNull()
    const button = screen.getByRole('button', { name: 'Activity' })
    expect(button.dataset.tour).toBe('rail-command-center')
    fireEvent.click(button)
    expect(screen.getByTestId('where').textContent).toBe('/command-center')
    expect(screen.getByRole('button', { name: 'Activity' }).getAttribute('aria-current')).toBe('page')
  })
})
