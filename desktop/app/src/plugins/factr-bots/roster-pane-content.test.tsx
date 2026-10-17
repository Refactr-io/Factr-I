import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { BOTS_LOCALES } from './i18n'
import { renderRosterContent } from './roster-pane-content'

const b = BOTS_LOCALES.en
const roster = (b as unknown as { roster: Record<string, string> }).roster

const none = () => null

/** The roster body for a given load state; every row renderer is unused on these three states. */
const content = (over: Partial<Parameters<typeof renderRosterContent>[0]>) =>
  renderRosterContent({
    allBotsHidden: false,
    b: b as never,
    error: null,
    gatewayGroups: [],
    gatewaySections: [],
    gatewayUp: true,
    hasRosterConstraint: false,
    hiddenBots: [],
    hiddenExpanded: false,
    hiddenGatewaySections: [],
    hiddenSectionRef: { current: null },
    initialRosterLoading: false,
    isLoading: false,
    matchingHiddenBots: [],
    query: '',
    refetch: vi.fn(),
    renderBotRow: none,
    renderGatewaySection: none,
    renderGroupChatSection: none,
    renderHiddenGatewaySection: none,
    renderUserSections: none,
    roster: [],
    rosterRows: [],
    selectedGateway: null,
    showGatewaySections: false,
    showHiddenRows: false,
    showHiddenSection: false,
    sortedGroupRows: [],
    staleNotice: null,
    ...over
  } as never)

describe('Bots list load states', () => {
  it('shows an error with Retry when the roster failed and there is nothing to show', () => {
    const refetch = vi.fn()

    render(<>{content({ error: new Error('The gateway did not answer in time'), refetch })}</>)

    expect(screen.getByText(/did not answer in time/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: roster.retryNow }))
    expect(refetch).toHaveBeenCalledTimes(1)
  })

  it('keeps showing the failure with Retry while a retry is in flight, never the bare loader', () => {
    render(<>{content({ error: new Error('Factr-I gateway is not connected'), isLoading: true })}</>)

    expect(screen.getByRole('button', { name: roster.retryNow })).toBeTruthy()
  })

  it('shows the loader only while nothing has failed yet', () => {
    const { container } = render(<>{content({ isLoading: true })}</>)

    expect(screen.queryByRole('button', { name: roster.retryNow })).toBeNull()
    expect(container.querySelector('.flex-1')).toBeTruthy()
  })

  it('says the gateway is not up, with Retry, when it is down', () => {
    render(<>{content({ error: new Error('x'), gatewayUp: false })}</>)

    expect(screen.getByText(roster.waitingForGateway)).toBeTruthy()
    expect(screen.getByRole('button', { name: roster.retryNow })).toBeTruthy()
  })

  it('shows the empty state once an answer arrives with no bots', () => {
    render(<>{content({})}</>)

    expect(screen.getByText(roster.emptyTitle)).toBeTruthy()
  })
})
