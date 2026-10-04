import { connectorTitle } from '@/lib/connector-tools'

import type {
  BundledEntryInput,
  ConnectorCardModel,
  ConnectorFact,
  ConnectorReason,
  ConnectorsFilter,
  ConnectorState,
  ConnectorStateWord,
  ConnectorVerb,
  ConnectorWayLocal,
  LocalServerInput,
  LocalServerStatus
} from './types'

export const EMPTY_CONNECTORS_FILTER: ConnectorsFilter = { query: '', segment: 'all' }

interface Phase {
  reason: ConnectorReason['key'] | undefined
  state: ConnectorState
  verb: ConnectorVerb | undefined
}

const LOCAL_PHASES = {
  error: { reason: 'serverError', state: 'broken', verb: 'openLogs' },
  'needs-auth': { reason: 'serverNeedsAuth', state: 'broken', verb: 'authenticate' },
  off: { reason: undefined, state: 'off', verb: undefined },
  ok: { reason: undefined, state: 'connected', verb: undefined },
  probing: { reason: undefined, state: 'connecting', verb: undefined },
  unknown: { reason: undefined, state: 'connecting', verb: undefined }
} satisfies Record<LocalServerStatus, Phase>

const LOCAL_WORDS = {
  available: 'available',
  broken: 'serverError',
  connected: 'serverOn',
  connecting: 'serverConnecting',
  expired: 'serverError',
  off: 'serverOff',
  unknown: 'serverConnecting'
} satisfies Record<ConnectorState, ConnectorStateWord>

function localFact(server: LocalServerInput, state: ConnectorState): ConnectorFact | undefined {
  if (state !== 'connected' || server.unused === true || server.toolsTotal === undefined) {
    return undefined
  }

  if (server.toolsOn === undefined) {
    return { count: server.toolsTotal, key: 'tools' }
  }

  return server.toolsOn < server.toolsTotal
    ? { count: server.toolsTotal, key: 'toolsSomeOn', on: server.toolsOn }
    : { count: server.toolsOn, key: 'toolsOn' }
}

export function localWay(server: LocalServerInput): ConnectorWayLocal {
  const status: LocalServerStatus = server.enabled ? server.status : 'off'
  const phase = LOCAL_PHASES[status]

  return {
    fact: localFact(server, phase.state),
    inCatalog: server.inCatalog,
    installed: true,
    plugin: server.plugin,
    reason: phase.reason ? { key: phase.reason } : undefined,
    serverEnabled: server.enabled,
    serverName: server.name,
    state: phase.state,
    target: server.target,
    unused: server.unused,
    verb: phase.verb === 'authenticate' && server.canAuthenticate === false ? 'openLogs' : phase.verb
  }
}

export function bundledWay(entry: BundledEntryInput): ConnectorWayLocal {
  return {
    authType: entry.authType,
    entryName: entry.name,
    inCatalog: true,
    installed: false,
    needsEnv: entry.needsEnv,
    state: 'available',
    verb: 'install'
  }
}

export function localWord(way: ConnectorWayLocal): ConnectorStateWord {
  if (way.reason?.key === 'serverNeedsAuth') {
    return 'serverNeedsAuth'
  }

  return way.state === 'connected' && way.unused === true ? 'serverOnUnused' : LOCAL_WORDS[way.state]
}

export interface CardInput {
  description?: string
  name: string
  slug: string
  way: ConnectorWayLocal
}

export function cardOfWay({ description, name, slug, way }: CardInput): ConnectorCardModel {
  return {
    description,
    fact: way.fact,
    inCatalog: way.inCatalog === true,
    name,
    plugin: way.plugin,
    reason: way.reason,
    slug,
    state: way.state,
    stateWord: localWord(way),
    verb: way.verb,
    way
  }
}

export function localServerName(card: ConnectorCardModel): string {
  return card.way.serverName ?? card.slug
}

export interface DeriveCardsInput {
  bundled?: readonly BundledEntryInput[]
  local: readonly LocalServerInput[]
}

export const localCardKey = (name: string) => `local:${name}`

export const cardKey = (card: ConnectorCardModel): string => localCardKey(card.slug)

// An installed server and the bundled entry of the same app share one card; the app's connector slug is the shared key.
const mergeKey = (connectorSlug: string | undefined, name: string) => connectorSlug ?? localCardKey(name)

export function deriveCards({ bundled = [], local }: DeriveCardsInput): ConnectorCardModel[] {
  const cards = new Map<string, ConnectorCardModel>()

  for (const server of local) {
    // An install must not rename the app: the bundled entry and the server it becomes read the same way.
    const title = server.title ?? connectorTitle(server.name)

    cards.set(
      mergeKey(server.connectorSlug, server.name),
      cardOfWay({ description: server.description, name: title, slug: server.name, way: localWay(server) })
    )
  }

  for (const entry of bundled) {
    const key = mergeKey(entry.connectorSlug, entry.name)

    // An installed server always beats the bundled entry of the same app.
    if (!cards.has(key)) {
      cards.set(
        key,
        cardOfWay({
          description: entry.description,
          name: connectorTitle(entry.name),
          slug: entry.name,
          way: bundledWay(entry)
        })
      )
    }
  }

  return [...cards.values()]
}
