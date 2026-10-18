/**
 * A bot's computer (one Docker container per bot, owned by the main process): a shared status store, the polling
 * that feeds it, and the small formatters the roster row and Screen pane print from. No window API (web or remote
 * gateway) means no store, no polling and nothing rendered.
 */

import { useEffect, useSyncExternalStore } from 'react'

import type { RosterRow } from './types'

const POLL_MS = 5000

type Api = NonNullable<Window['factrDesktop']['botComputer']>
export type ComputerStatus = Extract<Awaited<ReturnType<Api['status']>>, { ok: true }>['status']

const api = (): Api | undefined => (typeof window === 'undefined' ? undefined : window.factrDesktop?.botComputer)

/** Bots that have a computer (running or not), for the relay; empty where the desktop app has no such API. */
export async function listComputers(): Promise<{ profile: string; running: boolean }[]> {
  const result = await api()
    ?.list?.()
    .catch(() => null)

  return result?.ok ? result.computers : []
}

/** Only local, non-default bots get a computer, and only where the desktop app exposes the API. */
export const computerProfile = (bot: Partial<RosterRow> | null | undefined): null | string =>
  api() && bot?.name && bot.name !== 'default' && (!bot.connectionId || bot.connectionId === 'local') ? bot.name : null

const statuses = new Map<string, ComputerStatus>()
const watchers = new Map<string, number>()
const listeners = new Set<() => void>()
let timer: ReturnType<typeof setInterval> | undefined

function put(profile: string, status: ComputerStatus) {
  if (JSON.stringify(statuses.get(profile)) !== JSON.stringify(status)) {
    statuses.set(profile, status)
    listeners.forEach(fn => fn())
  }
}

async function poll(profile: string) {
  const result = await api()
    ?.status(profile)
    .catch(() => null)

  if (result?.ok) {
    put(profile, result.status)
  }
}

function tick() {
  if (!window.document.hidden) {
    watchers.forEach((_n, profile) => void poll(profile))
  }
}

function watch(profile: string) {
  watchers.set(profile, (watchers.get(profile) ?? 0) + 1)
  void poll(profile)
  timer ??= setInterval(tick, POLL_MS)

  return () => {
    const left = (watchers.get(profile) ?? 1) - 1

    if (left > 0) {
      watchers.set(profile, left)
    } else {
      watchers.delete(profile)
    }

    if (!watchers.size && timer) {
      clearInterval(timer)
      timer = undefined
    }
  }
}

const subscribe = (fn: () => void) => {
  listeners.add(fn)

  return () => void listeners.delete(fn)
}

async function run(profile: string, verb: 'start' | 'stop') {
  if (verb === 'start') {
    put(profile, { busy: false, state: 'starting' })
  }

  const result = await api()?.[verb](profile)

  put(
    profile,
    result?.ok ? result.status : { busy: false, error: result?.error ?? 'No answer', state: 'error' }
  )
}

export interface BotComputer {
  start: () => Promise<void>
  state: ComputerStatus['state']
  status: ComputerStatus
  stop: () => Promise<void>
}

/** The bot's computer, or null while unknown or unsupported. Polls while mounted. */
export function useBotComputer(bot: Partial<RosterRow> | null | undefined): BotComputer | null {
  const profile = computerProfile(bot)
  const status = useSyncExternalStore(subscribe, () => (profile ? statuses.get(profile) : undefined))

  useEffect(() => (profile ? watch(profile) : undefined), [profile])

  if (!profile || !status) {
    return null
  }

  return { start: () => run(profile, 'start'), state: status.state, status, stop: () => run(profile, 'stop') }
}

/** Keeps the computer awake while this pane is open (the main process stops idle computers). */
export function useComputerViewHold(bot: Partial<RosterRow> | null | undefined) {
  const profile = computerProfile(bot)

  useEffect(() => {
    if (!profile) {
      return
    }

    const hold = api()?.hold

    void hold?.(profile, 'view', true)

    return () => void hold?.(profile, 'view', false)
  }, [profile])
}

const WORD_KEY = {
  'no-docker': 'noDocker',
  off: 'off',
  on: 'on',
  starting: 'starting'
} as const

/** The roster's one status word key; Working replaces On while a view or turn holds the computer. */
export function computerWordKey(status: ComputerStatus): 'noDocker' | 'off' | 'on' | 'starting' | 'working' | null {
  if (status.state === 'error') {
    return null
  }

  return status.state === 'on' && status.busy ? 'working' : WORD_KEY[status.state]
}

const MIB = 1024 ** 2
const GIB = 1024 ** 3

export function formatMemory(bytes: number): string {
  return bytes >= GIB
    ? `${Number((bytes / GIB).toFixed(1))} GB`
    : `${Math.max(1, Math.round(bytes / MIB))} MB`
}

export function formatUptime(startedAt: string | undefined, now = Date.now()): string {
  const minutes = Math.max(0, Math.floor((now - Date.parse(startedAt ?? '')) / 60_000)) || 0
  const hours = Math.floor(minutes / 60)

  return hours ? `${hours} h ${minutes % 60} min` : `${minutes} min`
}

/** Teach by telling: the bot saves the task as a skill in its own profile. Sent to a model, so English by design. */
export const TEACH_TEMPLATE = `Learn this task and save it as a skill named <name> in your own profile.
Steps:
1. <first step>
2. <next step>
Inputs: <what I will give you each time>
Success check: <how you will know it worked>
Schedule (optional): <for example every weekday at 9am>
When I say 'run <name>', do it on your own computer and check it worked.`
