// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { MemoryRouter, useLocation } from 'react-router'
import { afterEach, describe, expect, it } from 'vitest'

import { SidebarProvider } from '@/components/ui/sidebar'
import { ar } from '@/i18n/ar'
import { en } from '@/i18n/en'
import { ja } from '@/i18n/ja'
import { ru } from '@/i18n/ru'
import { zh } from '@/i18n/zh'
import { zhHant } from '@/i18n/zh-hant'
import { shownInMode } from '@/store/interface-mode'

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

describe('Run history nav entry', () => {
  it('is an advanced-tier row pointing at the agents page, after cron', () => {
    const ids = SIDEBAR_NAV.map(item => item.id)
    const item = SIDEBAR_NAV.find(entry => entry.id === 'agents')

    expect(ids.indexOf('agents')).toBe(ids.indexOf('cron') + 1)
    expect(item).toMatchObject({ keybindActionId: 'nav.agents', route: AGENTS_ROUTE, tier: 'advanced' })
    expect(SIDEBAR_NAV.filter(shownInMode('simple')).some(entry => entry.id === 'agents')).toBe(false)
    expect(SIDEBAR_NAV.filter(shownInMode('advanced')).some(entry => entry.id === 'agents')).toBe(true)
    expect(en.sidebar.nav.agents).toBe('Run history')
  })

  it('renders a Run history rail button that navigates to /agents and marks itself current', () => {
    renderRail('/')
    const button = screen.getByRole('button', { name: 'Run history' })

    expect(button.getAttribute('aria-current')).toBeNull()
    fireEvent.click(button)

    expect(screen.getByTestId('where').textContent).toBe(AGENTS_ROUTE)
    expect(screen.getByRole('button', { name: 'Run history' }).getAttribute('aria-current')).toBe('page')
  })
})

describe('Run history locale parity', () => {
  it.each([ar, ja, ru, zh, zhHant])('translates the nav label and command center strings', locale => {
    expect(locale.sidebar.nav.agents).not.toBe(en.sidebar.nav.agents)
    expect(locale.commandCenter.runHistory).not.toBe(en.commandCenter.runHistory)
    expect(locale.commandCenter.railTip).not.toBe(en.commandCenter.railTip)
  })
})
