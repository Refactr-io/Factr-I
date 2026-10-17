import type * as FactrSdk from '@factr/plugin-sdk'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { BotRow } from './bot-row'
import { BotChatEmpty } from './chat-empty'
import { type ComputerStatus, computerWordKey, formatMemory, formatUptime, TEACH_TEMPLATE } from './computer'
import { $lastRoster } from './data'
import { translateBots, translateBotsIn } from './i18n-test-helper'
import type * as ScreenConnection from './screen-connection'
import { BotScreenPane } from './screen-pane'
import type { RosterRow } from './types'

const { displayRequest, focus, insert } = vi.hoisted(() => ({ displayRequest: vi.fn(), focus: vi.fn(), insert: vi.fn() }))

vi.mock('@factr/plugin-sdk', async importOriginal => {
  const sdk = await importOriginal<typeof FactrSdk>()

  return { ...sdk, requestComposerFocus: focus, requestComposerInsert: insert, usePluginI18n: () => translateBotsIn('en') }
})

vi.mock('./screen-connection', async importActual => ({
  ...(await importActual<typeof ScreenConnection>()),
  displayRequest
}))

const t = (key: string, ...args: unknown[]) => translateBots(key, ...args)
const MIB = 1024 ** 2

const api = {
  hold: vi.fn(async () => ({ ok: true })),
  openDocker: vi.fn(async () => ({ ok: true })),
  start: vi.fn(),
  status: vi.fn(),
  stop: vi.fn()
}

const openExternal = vi.fn(async () => undefined)
let n = 0
/** A fresh profile per test: the status store is module state shared across renders. */
const newBot = (): RosterRow => ({ connectionId: 'local', name: `bot${++n}` }) as RosterRow

const answer = (status: Partial<ComputerStatus>) =>
  api.status.mockResolvedValue({ ok: true, status: { busy: false, state: 'off', ...status } })

const unsupported = { installed: false, running: false, supported: false }
const stopped = { installed: true, running: false, supported: true }

beforeEach(() => {
  vi.clearAllMocks()
  window.factrDesktop = { botComputer: api, openExternal } as never
  displayRequest.mockResolvedValue(unsupported)
})

afterEach(() => {
  delete (window as { factrDesktop?: unknown }).factrDesktop
})

describe('status word', () => {
  it('maps every state to one word', () => {
    const word = (status: Partial<ComputerStatus>) => computerWordKey({ busy: false, state: 'off', ...status })

    expect(word({ state: 'off' })).toBe('off')
    expect(word({ state: 'starting' })).toBe('starting')
    expect(word({ state: 'on' })).toBe('on')
    expect(word({ busy: true, state: 'on' })).toBe('working')
    expect(word({ state: 'no-docker' })).toBe('noDocker')
    expect(word({ state: 'error' })).toBeNull()
    expect(t('computer.word.noDocker')).toBe('No Docker')
    expect(t('computer.word.working')).toBe('Working')
  })

  it('formats uptime and memory', () => {
    const now = Date.parse('2026-01-01T01:05:00Z')

    expect(formatUptime('2026-01-01T00:53:00Z', now)).toBe('12 min')
    expect(formatUptime('2025-12-31T23:00:00Z', now)).toBe('2 h 5 min')
    expect(formatMemory(640 * MIB)).toBe('640 MB')
    expect(formatMemory(2 * 1024 * MIB)).toBe('2 GB')
  })

  it('shows on a local bot row, and nowhere without the API or for a remote bot', async () => {
    answer({ busy: true, state: 'on' })
    const bot = newBot()
    const view = render(<BotRow bot={bot} onDelete={vi.fn()} onEdit={vi.fn()} onGroup={vi.fn()} onNewSection={vi.fn()} />)

    expect(await screen.findByText('Working')).toBeTruthy()
    view.unmount()

    const remote = { ...newBot(), connectionId: 'work' }
    render(<BotRow bot={remote} onDelete={vi.fn()} onEdit={vi.fn()} onGroup={vi.fn()} onNewSection={vi.fn()} />)
    expect(api.status).toHaveBeenCalledTimes(1)
    expect(screen.queryByText('Off')).toBeNull()
  })

  it('renders and polls nothing when the API is absent', () => {
    delete (window.factrDesktop as { botComputer?: unknown }).botComputer
    render(<BotRow bot={newBot()} onDelete={vi.fn()} onEdit={vi.fn()} onGroup={vi.fn()} onNewSection={vi.fn()} />)

    expect(api.status).not.toHaveBeenCalled()
    expect(screen.queryByText('Off')).toBeNull()
  })
})

describe('Screen pane', () => {
  it('offers Start when the computer is off, and starts it', async () => {
    answer({ state: 'off' })
    api.start.mockResolvedValue({ ok: true, status: { busy: false, state: 'starting' } })
    render(<BotScreenPane bot={newBot()} />)

    fireEvent.click(await screen.findByRole('button', { name: t('computer.start') }))

    expect(api.start).toHaveBeenCalledTimes(1)
    expect(await screen.findByText(t('computer.starting'))).toBeTruthy()
  })

  it('shows uptime, memory and Stop when the computer is on', async () => {
    displayRequest.mockResolvedValue(stopped)
    answer({
      memLimitBytes: 2048 * MIB,
      memUsedBytes: 640 * MIB,
      startedAt: new Date(Date.now() - 12 * 60_000 - 5000).toISOString(),
      state: 'on'
    })
    api.stop.mockResolvedValue({ ok: true, status: { busy: false, state: 'off' } })
    render(<BotScreenPane bot={newBot()} />)

    expect(await screen.findByText('On for 12 min, using 640 MB of 2 GB')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: t('computer.stop') }))
    await waitFor(() => expect(api.stop).toHaveBeenCalledTimes(1))
    expect(await screen.findByRole('button', { name: t('computer.start') })).toBeTruthy()
  })

  it('says Docker is not running and opens Docker on a Mac', async () => {
    Object.defineProperty(navigator, 'platform', { configurable: true, value: 'MacIntel' })
    answer({ reason: 'not-running', state: 'no-docker' })
    render(<BotScreenPane bot={newBot()} />)

    expect(await screen.findByText('Docker is not running')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Open Docker' }))
    expect(api.openDocker).toHaveBeenCalledTimes(1)
    expect(openExternal).not.toHaveBeenCalled()
    Object.defineProperty(navigator, 'platform', { configurable: true, value: '' })
  })

  it('says Docker is not installed and links to the download', async () => {
    answer({ reason: 'not-installed', state: 'no-docker' })
    render(<BotScreenPane bot={newBot()} />)

    expect(await screen.findByText('Docker is not installed')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Get Docker' }))
    expect(openExternal).toHaveBeenCalledWith('https://www.docker.com/products/docker-desktop/')
  })

  it('holds the computer while mounted and releases it on unmount', async () => {
    answer({ state: 'on' })
    const bot = newBot()
    const view = render(<BotScreenPane bot={bot} />)

    expect(api.hold).toHaveBeenCalledWith(bot.name, 'view', true)
    view.unmount()
    expect(api.hold).toHaveBeenLastCalledWith(bot.name, 'view', false)
  })

  it('adds nothing without the API', async () => {
    delete (window.factrDesktop as { botComputer?: unknown }).botComputer
    render(<BotScreenPane bot={newBot()} />)

    expect(await screen.findByText('Bot screens run on Linux gateway hosts.', { exact: false })).toBeTruthy()
    expect(screen.queryByText(t('computer.start'))).toBeNull()
    expect(api.hold).not.toHaveBeenCalled()
  })
})

describe('Teach a task', () => {
  const chat = (bot: RosterRow) => {
    act(() => $lastRoster.set([{ ...bot, canonical_session: { id: 's1' } }] as never))

    return render(<BotChatEmpty sessionId="s1" />)
  }

  it('inserts the template into the composer', () => {
    chat(newBot())
    fireEvent.click(screen.getByRole('button', { name: 'Teach a task' }))

    expect(insert).toHaveBeenCalledWith(TEACH_TEMPLATE)
    expect(focus).toHaveBeenCalledTimes(1)
    expect(TEACH_TEMPLATE).not.toMatch(/[\u2013\u2014]/)
    expect(TEACH_TEMPLATE).toContain('save it as a skill')
  })

  it('is absent without the API', () => {
    delete (window.factrDesktop as { botComputer?: unknown }).botComputer
    chat(newBot())

    expect(screen.queryByRole('button', { name: 'Teach a task' })).toBeNull()
  })
})
