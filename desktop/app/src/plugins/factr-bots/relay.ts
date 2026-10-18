/**
 * The cross-connection bot relay: the Desktop-as-router loops that let a bot
 * on one gateway reach a bot on another.
 *
 * There is one relay per renderer, so its lifecycle lives in the module-scoped
 * `relay` record below rather than in an atom — no UI reads it. plugin.tsx
 * drives the two doors, startBotRelay / stopBotRelay.
 */

import { host, LruCache } from '@factr/plugin-sdk'

import { listComputers } from './computer'
import { botHandle, clearBotAttention, noteBotAttention } from './data'
import type { ProfileRoute, RosterRow } from './types'

// ── cross-connection bot relay ────────────────────────────────────────────
// Connections ARE the peer set: every gateway this Desktop holds a socket
// to (local, remote URL, SSH, Factr Cloud, docker) must be able to find
// every other connection's agents and message them via message_agent. The
// Desktop is the relay — it owns every socket. Two loops:
//  - roster loop: pushes each gateway the union roster of agents on the
//    OTHER connections (bot_relay.roster.sync), so message_agent resolves
//    cross-connection targets and Bot Chat prompts list them;
//  - drain loop: collects queued envelopes from every gateway
//    (bot_relay.outbox.drain), delivers each on the target connection's
//    own socket (bot_relay.deliver), and posts the reply back to the
//    sender gateway (bot_relay.reply) where a waiter wakes the sender.
// Older backends without the RPCs fail per-call and are skipped — the
// relay degrades to whatever subset of connections supports it.
const RELAY_ROSTER_INTERVAL_MS = 60_000
// Backstop cadence only (#93594): the push path below carries envelope latency,
// so the interval poll exists for older backends and missed events — 30s
// matches LIVE_SESSION_STATUS_BACKSTOP_INTERVAL_MS. It was 4s back when the
// poll WAS the delivery path, which (before route retention) also meant a
// fresh WebSocket dial + teardown per registered connection every 4s.
const RELAY_DRAIN_INTERVAL_MS = 30_000
// #93911: a delivered turn runs on the target gateway, so the client must
// outlive the backend's own bound. Without this the call fell to the pool's
// generic 30s deadline and every long turn (Computer Use, deep research) came
// back as an unclassified failure.
//
// The backend's MAXIMUM WORK budget is spelled out below. The client deadline
// must be strictly GREATER than it: after those bounded waits the handler still
// has to classify the failure, build and run the retry, classify/serialize the
// terminal result, unwind the temp-file and lock scopes, and get the JSON-RPC
// response back through the event loop. A call that consumes nearly all of the
// work budget would otherwise lose the race to this timer by milliseconds and
// reproduce #93911 at the upper boundary — the backend knowing a typed reason
// while Desktop reports its generic timeout first.
//
// These three are mirrors of backend values, so a change there must not
// silently invalidate this constant: relay-deliver-budget.test.ts reads
// factr_backend/config_defaults.py and tools/bot_relay.py and fails if the
// mirrors drift or the margin stops being positive.
const RELAY_TURN_LOCK_WAIT_MS = 120_000 // bot_mode.turn_wait_seconds default
const RELAY_TURN_ATTEMPT_MS = 600_000 // tools/bot_relay.py TURN_ATTEMPT_TIMEOUT_SECONDS
const RELAY_TURN_MAX_ATTEMPTS = 2 // first attempt + the policy-gated re-run

const RELAY_DELIVER_BACKEND_CEILING_MS = RELAY_TURN_LOCK_WAIT_MS + RELAY_TURN_ATTEMPT_MS * RELAY_TURN_MAX_ATTEMPTS

// Settlement + transport headroom on top of the ceiling, so a backend that
// answers at its own limit still wins the race against this timer.
const RELAY_DELIVER_SETTLEMENT_MARGIN_MS = 180_000
// tools/bot_relay.py REPLY_WAIT_SECONDS rebuilds this sum and waits past it for the timeout reply below.
const RELAY_DELIVER_TIMEOUT_MS = RELAY_DELIVER_BACKEND_CEILING_MS + RELAY_DELIVER_SETTLEMENT_MARGIN_MS
// Push path (#93091): the gateway broadcasts `bot_relay.outbox.pending` when
// an envelope lands on disk; a burst of signals inside this window collapses
// to ONE drain. The interval poll above stays as the backstop for older
// backends (and connections whose events don't reach the tap).
const RELAY_PUSH_DEBOUNCE_MS = 250
// A message between two bots on THIS install (target connection 'local') is delivered by the target bot's own engine
// (`bot.chat.deliver`: a real turn in its Bot Chat), not a Python CLI turn. The Desktop is the only router, so while the
// relay runs it stamps a heartbeat the Python tools check (bot_relay.relay_alive) and watches the outbox cheaply (a
// file count in the main process, never waking a backend) so one queued message is drained at once.
const RELAY_LOCAL_TICK_MS = 4000
// A relayed turn can take minutes: `bot.chat.deliver` answers `pending` after ~45 s and the relay polls
// `bot.chat.deliver_status`, so no single call outlives a request deadline.
const RELAY_ENGINE_CALL_MS = 130_000
// A bot with its own computer is its own relay connection: its engine is a separate backend with its own outbox,
// roster and turn lock, and the local connection must never carry it. Mirrors tools/bot_relay.py
// COMPUTER_CONNECTION_PREFIX (connection ids are `[A-Za-z0-9_-]{1,64}`, so a profile over 55 chars has no route).
const COMPUTER_CONNECTION_PREFIX = 'computer-'
const computerConnectionId = (profile: string) => `${COMPUTER_CONNECTION_PREFIX}${profile}`
// The Rust engine holds a forwarded RPC for at most 120 s, so a relayed turn is delivered by polling: each
// bot_relay.deliver call returns within the backend's own poll window (90 s) with `pending` until the turn is done.
const RELAY_DELIVER_POLL_TIMEOUT_MS = 150_000

/** Everything the two loops mutate. */
interface RelayLifecycle {
  disposed: boolean
  drainBusy: boolean
  /** A push landing while a drain is ALREADY running would be lost forever —
   *  the gateway signature is monotone (one event per new envelope, never
   *  re-broadcast) — so remember it and re-schedule after the drain finishes. */
  drainRerun: boolean
  drainTimer: null | ReturnType<typeof setInterval>
  /** The local outbox had mail at the last tick: the next drain runs even with a single connection. */
  localPending: boolean
  localTimer: null | ReturnType<typeof setInterval>
  pushDebounceTimer: null | ReturnType<typeof setTimeout>
  pushUnsub: (() => void) | null
  rosterBusy: boolean
  /** Id of the sole connection whose roster clear went out; null once the peer set relays again. */
  rosterClearedFor: null | string
  rosterTimer: null | ReturnType<typeof setInterval>
}

const relay: RelayLifecycle = {
  disposed: false,
  drainBusy: false,
  drainRerun: false,
  drainTimer: null,
  localPending: false,
  localTimer: null,
  pushDebounceTimer: null,
  pushUnsub: null,
  rosterBusy: false,
  rosterClearedFor: null,
  rosterTimer: null
}

// Relay-route socket retention (#93594): connection id → release fn. While
// the relay is active each registered connection's pooled socket is pinned
// open (host.retainProfileSocket) so drain RPCs reuse ONE persistent
// WebSocket instead of dialing and tearing down a fresh one per tick.
// Feature-detected — older shells lack the door and fall back to per-call
// leases. Local routes get a no-op release inside the host (idle-reaper
// exemption). stopBotRelay releases everything.
const relayRouteRetentions = new Map<string, () => void>()

/** One reachable gateway plus a representative route onto it. The route carries
 *  identity only, so the human label comes from the registry (connectionLabels). */
interface RelayConnection {
  id: string
  route: ProfileRoute
  /** A computer that is off is a delivery target (delivering wakes it) but is never polled or pushed to. */
  live: boolean
  /** The bot whose computer this connection is. */
  computerProfile?: string
}

/** One agent as pushed to a peer gateway's relay roster. */
interface RelayAgentRow {
  connection_id: string
  connection_label: string
  description: string
  handle: string
  profile: string
  title: string
}

/** A queued cross-connection message drained from a gateway's outbox. */
interface RelayEnvelope {
  native?: boolean
  sender_session?: string
  id?: string
  message?: string
  from_profile?: string
  from_handle?: string
  hops?: number
  target_connection?: string
  target_profile?: string
}

/** Reconcile retention with the CURRENT connection set: pin new connections,
 *  release removed ones. Runs on every drain/roster connection fetch. */
function syncRelayRetention(connections: RelayConnection[]) {
  if (typeof host.retainProfileSocket !== 'function') {
    return
  }

  const live = new Set(connections.map(connection => connection.id))

  for (const [id, release] of [...relayRouteRetentions]) {
    if (!live.has(id)) {
      relayRouteRetentions.delete(id)

      try {
        release()
      } catch {
        // Never let a release failure break the relay loop.
      }
    }
  }

  if (relay.disposed) {
    return
  }

  for (const connection of connections) {
    if (!relayRouteRetentions.has(connection.id)) {
      relayRouteRetentions.set(connection.id, host.retainProfileSocket(connection.route))
    }
  }
}

/** Drop every relay pin — stop/dispose path. */
function releaseRelayRetention() {
  for (const release of relayRouteRetentions.values()) {
    try {
      release()
    } catch {
      // Disposer from an older shell shape — never break teardown.
    }
  }

  relayRouteRetentions.clear()
}

/** One representative route per reachable connection id. */
async function relayConnections(): Promise<RelayConnection[]> {
  if (typeof host.profileRoutes !== 'function' || typeof host.requestProfile !== 'function') {
    return []
  }

  try {
    const fetched = await host.profileRoutes()
    const routes = Array.isArray(fetched) ? fetched : []
    const computers = await listComputers()
    const computerProfiles = new Set(computers.map(computer => computer.profile))
    const byConnection = new Map<string, ProfileRoute>()

    for (const route of routes) {
      const id = String(route?.connectionId || '')

      // A computer's profile is never the representative of the local connection: a call on its route wakes it.
      const ownedByComputer = id === 'local' && computerProfiles.has(String(route?.profile || ''))

      if (id && !ownedByComputer && !byConnection.has(id)) {
        byConnection.set(id, route)
      }
    }

    const connections: RelayConnection[] = [...byConnection.entries()].map(([id, route]) => ({
      id,
      live: true,
      route
    }))

    for (const { profile, running } of computers) {
      if (computerConnectionId(profile).length > 64) {
        continue
      }

      connections.push({
        computerProfile: profile,
        id: computerConnectionId(profile),
        live: running,
        route: routes.find(route => route?.connectionId === 'local' && route?.profile === profile) ?? {
          connectionId: 'local',
          mode: 'local',
          profile,
          targetProfile: profile
        }
      })
    }

    return connections
  } catch {
    return []
  }
}

/** Human label per connection id, from the registry — the only place that has one.
 *  A `host.profileRoutes()` route carries identity alone, so without this the roster a
 *  peer gateway shows its bots ("@chii on 127-0-0-1-9119", tools/bot_mode_probe.py) and
 *  its refusal text name machines by raw connection id. A build with no registry rejects
 *  the call, and ids stay. */
async function connectionLabels(): Promise<Map<string, string>> {
  if (typeof host.connections !== 'function') {
    return new Map()
  }

  try {
    const rows = await host.connections()

    return new Map(
      (Array.isArray(rows) ? rows : [])
        .map(row => [String(row?.id || ''), String(row?.label || '').trim()] as const)
        .filter(([id, label]) => Boolean(id && label))
    )
  } catch {
    return new Map()
  }
}

/** The agents living on one connection, as relay roster rows.
 *  Returns null on FAILURE (transient RPC blip, slow socket) — distinct from
 *  a genuine empty profile list. Conflating the two would push a fresh union
 *  roster missing a LIVE connection's agents, and the gateway-side liveness
 *  check (bot_relay._target_liveness) reads "absent from a fresh roster" as
 *  definitively offline → false runtime_offline refusals (#93091 item 2). */
async function relayAgentsOn(
  connection: RelayConnection,
  labels: Map<string, string>,
  computerProfiles: Set<string> = new Set()
): Promise<RelayAgentRow[] | null> {
  try {
    const res = await host.requestProfile<{ profiles?: RosterRow[] }>(connection.route, 'profiles.list', {
      include_sessions: false
    })

    const profiles = Array.isArray(res?.profiles) ? res.profiles : []
    const label = labels.get(connection.id) || connection.id

    return profiles
      .map(profile => ({
        profile: String(profile?.name || ''),
        handle: botHandle(profile?.name, profile),
        // A bot with a computer lives on its own connection, wherever the local profile list reports it.
        ...(connection.id === 'local' && computerProfiles.has(String(profile?.name || ''))
          ? {
              connection_id: computerConnectionId(String(profile?.name || '')),
              connection_label: `${botHandle(profile?.name, profile)} computer`
            }
          : { connection_id: connection.id, connection_label: label }),
        title: String(profile?.ui_meta?.['factr-bots']?.title || profile?.display_name || ''),
        description: String(profile?.description || '')
      }))
      .filter(row => row.profile)
  } catch {
    return null
  }
}

/** Last good agent rows per connection id — reused when a fetch blips so a
 *  transient failure never reads as "everyone on that machine went away".
 *  The sweep below drops disconnected ids, but only on a cycle that had two
 *  or more connections to relay between; the ceiling is what bounds the rest.
 *  Every live connection is rewritten each cycle, so eviction can only ever
 *  reach ids that stopped being fetched. */
const RELAY_AGENTS_CACHE_MAX = 32
const relayAgentsCache = new LruCache<string, RelayAgentRow[]>(RELAY_AGENTS_CACHE_MAX)

/** Push every gateway the union roster of agents on the OTHER connections. */
async function syncRelayRosters() {
  if (relay.disposed || relay.rosterBusy) {
    return
  }

  relay.rosterBusy = true

  try {
    const connections = await relayConnections()
    const labels = await connectionLabels()

    if (connections.length < 2) {
      // Nothing to relay — but the gateways that remain still hold the last
      // pushed roster, so a departed machine's agents would stay in every
      // bot's prompt (and as message_agent targets) until a second connection
      // reappears. Push the now-empty roster once per sole connection so it
      // forgets it — a replacement sole connection has never been told. An
      // empty route list (registry not loaded yet) must not spend the clear.
      if (connections.length === 1 && connections[0].id !== relay.rosterClearedFor) {
        const cleared = await Promise.all(
          connections.map(async connection => {
            try {
              await host.requestProfile(connection.route, 'bot_relay.roster.sync', { agents: [] })

              return true
            } catch {
              // An older backend without the relay RPCs, but also a socket that is not up yet —
              // requestProfile throws for both. Either way this gateway still holds the departed
              // machine's agents, so the clear is NOT spent and the next tick tries again.
              return false
            }
          })
        )

        // Spend it only once every gateway has actually forgotten.
        if (cleared.every(Boolean)) {
          relay.rosterClearedFor = connections[0].id
        }
      }

      return
    }

    relay.rosterClearedFor = null

    const computerProfiles = new Set(connections.flatMap(connection => connection.computerProfile ?? []))
    const rows: RelayAgentRow[] = []
    await Promise.all(
      // A computer contributes no fetch of its own: its row comes from the local profile list (a fetch would wake it).
      connections
        .filter(connection => !connection.computerProfile)
        .map(async connection => {
          const agents = await relayAgentsOn(connection, labels, computerProfiles)

          if (agents === null) {
            // Transient fetch failure: reuse the last good rows for this
            // connection (or contribute nothing this cycle) so the pushed
            // roster never drops a live machine's agents — absence from a
            // fresh roster means offline to the gateway-side fail-fast.
            rows.push(...(relayAgentsCache.get(connection.id) || []))
          } else {
            relayAgentsCache.set(connection.id, agents)
            rows.push(...agents)
          }
        })
    )

    // Connections gone from profileRoutes are genuinely disconnected — drop
    // their cache so a later reconnect starts from live data.
    const liveIds = new Set(connections.map(connection => connection.id))

    for (const id of [...relayAgentsCache.keys()]) {
      if (!liveIds.has(id)) {
        relayAgentsCache.delete(id)
      }
    }

    // Only live connections are pushed to: a computer that is off has no engine to push into (and a push wakes it).
    await Promise.all(
      connections
        .filter(connection => connection.live)
        .map(async connection => {
          const others = rows.filter(row => row.connection_id !== connection.id)

          try {
            const res = await host.requestProfile<{ count?: number }>(connection.route, 'bot_relay.roster.sync', {
              agents: others
            })

            // A freshly started engine answers roster.sync from a boot stub ({count: 0}) until its Python backend
            // runs. Any real drain starts it; claim what is queued (it is delivered like any drain) and push again.
            if (connection.computerProfile && others.length > 0 && res?.count === 0) {
              const byId = new Map(connections.map(peer => [peer.id, peer]))

              for (const { envelope, sender } of await claimFrom(connection)) {
                enqueueRelayDelivery(sender, envelope, byId)
              }

              await host.requestProfile(connection.route, 'bot_relay.roster.sync', { agents: others })
            }
          } catch {
            // Older backend without the relay RPCs — skip this connection.
          }
        })
    )
  } finally {
    relay.rosterBusy = false
  }
}

/** Claim one connection's queued envelopes (an unreachable or older gateway yields none). */
async function claimFrom(sender: RelayConnection): Promise<RelayQueuedEnvelope[]> {
  try {
    const res = await host.requestProfile<{ envelopes?: RelayEnvelope[] }>(sender.route, 'bot_relay.outbox.drain', {})

    return (Array.isArray(res?.envelopes) ? res.envelopes : []).map(envelope => ({ envelope, sender }))
  } catch {
    // Older backend without the relay RPCs — skip this connection.
    return []
  }
}

/** Drain every gateway's outbox and deliver each envelope on the target
 *  connection's own socket; the reply (or error) is posted back to the
 *  sender gateway for its waiter. */
async function drainRelayOutboxes() {
  if (relay.disposed) {
    return
  }

  if (relay.drainBusy) {
    // A push signal raced an in-flight drain. The gateway never re-sends it
    // (monotone signature), so without this flag the envelope would wait out
    // the full poll interval — exactly the latency the push path removes.
    relay.drainRerun = true

    return
  }

  relay.drainBusy = true
  const forced = relay.localPending

  relay.localPending = false

  try {
    const connections = await relayConnections()

    // Retention follows the relay-eligible set: with fewer than two
    // connections there is nothing to relay, so nothing stays pinned.
    syncRelayRetention(connections.length >= 2 ? connections.filter(connection => connection.live) : [])

    // One connection is nothing to relay BETWEEN, unless local bots have mail for each other (forced by the local tick).
    if (connections.length < (forced ? 1 : 2)) {
      return
    }

    const byId = new Map(connections.map(connection => [connection.id, connection]))

    // Phase 1 — claim every gateway's outbox before delivering anything. The
    // gateway checks an envelope's age against bot_mode.envelope_ttl_seconds
    // AT the claim, so a claim must never wait behind another gateway's
    // delivery turn: draining and delivering sender by sender let one long
    // turn (up to RELAY_DELIVER_TIMEOUT_MS) age a sibling gateway's envelope
    // past the TTL before it was even claimed, and that sender's waiter got
    // `queued_expired` for a message nothing was wrong with.
    const queued: RelayQueuedEnvelope[] = []

    // A computer that is off has nothing queued and a drain would wake it: only live connections are claimed from.
    for (const sender of connections.filter(connection => connection.live)) {
      queued.push(...(await claimFrom(sender)))
    }

    // Phase 2 — deliver, without holding the drain. Envelopes for the same
    // target profile stay in order, one turn at a time (the target gateway
    // serialises that profile's turns behind its turn lock anyway; a second
    // envelope sent while the first runs would only burn its lock-wait
    // budget). Different targets run concurrently, so one long turn never
    // holds every other bot's mail. The lanes outlive this drain: `drainBusy`
    // covers the claim only, so a push that lands while a turn runs claims
    // its envelope NOW instead of after that turn — otherwise the TTL clock
    // kept running against a message the Desktop had not even looked at.
    for (const { envelope, sender } of queued) {
      enqueueRelayDelivery(sender, envelope, byId)
    }
  } finally {
    relay.drainBusy = false

    if (relay.drainRerun && !relay.disposed) {
      // Envelopes signaled mid-drain: schedule one follow-up pass (debounced)
      // instead of leaving them to the interval poll.
      relay.drainRerun = false
      scheduleRelayPushDrain()
    }
  }
}

interface RelayQueuedEnvelope {
  envelope: RelayEnvelope
  sender: RelayConnection
}

// Delivery lanes: `target_connection::target_profile` → the tail of that
// target's in-flight deliveries. A lane entry is removed once its tail
// settles so an idle target holds no state.
const relayLanes = new Map<string, Promise<void>>()

/** Queue one claimed envelope behind the deliveries already running for the
 *  same target profile; other targets are untouched. */
function enqueueRelayDelivery(sender: RelayConnection, envelope: RelayEnvelope, byId: Map<string, RelayConnection>) {
  const key = `${String(envelope?.target_connection || '')}::${String(envelope?.target_profile || '')}`

  const tail = (relayLanes.get(key) ?? Promise.resolve()).then(() =>
    relay.disposed ? undefined : deliverRelayEnvelope(sender, envelope, byId)
  )

  relayLanes.set(key, tail)
  void tail.finally(() => {
    if (relayLanes.get(key) === tail) {
      relayLanes.delete(key)
    }
  })
}

const isUnknownMethod = (error: any) =>
  /unknown method|method not found|not supported|-32601/i.test(String(error?.message || error))

/** One relayed message, delivered by the target bot's OWN engine (`bot.chat.deliver`) when it has that door, else by the
 *  gateway's `bot_relay.deliver` (a Python CLI turn: older engines, remote gateways). */
async function deliverToTarget(
  target: RelayConnection,
  sender: RelayConnection,
  envelope: RelayEnvelope,
  envelopeId: string
): Promise<{ reply?: string }> {
  const profile = String(envelope?.target_profile || '')
  const message = String(envelope?.message || '')
  const from_profile = String(envelope?.from_profile || '')
  const from_handle = String(envelope?.from_handle || '')
  const from_connection = String(sender.id)

  // Route to the bot's own engine: same connection, the bot's own profile.
  const route: ProfileRoute = { ...target.route, profile, targetProfile: profile }
  const deadline = Date.now() + RELAY_DELIVER_TIMEOUT_MS

  const request = async (method: string, params: Record<string, unknown>) => {
    for (;;) {
      try {
        return await host.requestProfile<EngineDelivery>(route, method, params, RELAY_ENGINE_CALL_MS, {
          spawnPriority: 'foreground'
        })
      } catch (error: any) {
        if (error?.data?.reason !== 'retiring' || Date.now() >= deadline) throw error
        await new Promise(resolve => setTimeout(resolve, 150))
      }
    }
  }
  try {
    let res = await request('bot.chat.deliver', {
      profile,
      text: message,
      from_handle,
      from_profile,
      from_connection,
      envelope_id: envelopeId,
      hops: envelope?.hops ?? 0
    })

    while (res?.status === 'pending') {
      if (Date.now() > deadline) {
        throw Object.assign(new Error('delivery timed out'), { data: { reason: 'delivery_timeout' } })
      }

      res = await request('bot.chat.deliver_status', { envelope_id: envelopeId })
    }

    if (res?.ok === false) {
      throw Object.assign(new Error(String(res.error || 'delivery failed')), { data: { reason: res.reason } })
    }

    return { reply: String(res?.reply || '') }
  } catch (error: any) {
    if (!isUnknownMethod(error)) {
      throw error
    }
  }

  const base = { profile, message, from_profile, from_handle, from_connection }
  for (;;) {
    let res: { pending?: boolean; reply?: string }
    try {
      res = await host.requestProfile(route, 'bot_relay.deliver', { ...base, hops: envelope.hops ?? 0, envelope_id: envelopeId }, RELAY_DELIVER_POLL_TIMEOUT_MS)
    } catch (error: any) {
      if (!/extra inputs are not permitted|out of sync/i.test(String(error?.message || error))) throw error
      return host.requestProfile(route, 'bot_relay.deliver', base, RELAY_DELIVER_TIMEOUT_MS)
    }
    if (!res?.pending) return res
    if (Date.now() > deadline) throw Object.assign(new Error('delivery timed out'), { data: { reason: 'delivery_timeout' } })
  }
}

interface EngineDelivery {
  error?: string
  ok?: boolean
  reason?: string
  reply?: string
  status?: string
}

/** Deliver one claimed envelope on the target connection's own socket and post
 *  the reply (or the error) back to the sender gateway for its waiter. */
async function deliverRelayEnvelope(
  sender: RelayConnection,
  envelope: RelayEnvelope,
  byId: Map<string, RelayConnection>
) {
  const envelopeId = String(envelope?.id || '')
  const target = byId.get(String(envelope?.target_connection || ''))

  const postReply = async (payload: { error?: string; reason?: string; reply?: string }) => {
    try {
      // Persist first: the native engine recovers this wake after a crash or pool retirement.
      await host.requestProfile(sender.route, 'bot_relay.reply', { id: envelopeId, ...payload })
      if (envelope.native === true && envelope.sender_session) {
        const profile = String(envelope.from_profile || 'default')
        const route = { ...sender.route, profile, targetProfile: profile }
        let reply = await host.requestProfile<EngineDelivery>(
          route,
          'bot.chat.reply',
          { id: envelopeId, ...payload },
          RELAY_ENGINE_CALL_MS,
          { spawnPriority: 'foreground' }
        )
        const deadline = Date.now() + RELAY_DELIVER_TIMEOUT_MS
        while (reply?.status === 'pending') {
          if (Date.now() > deadline) throw new Error('sender wake timed out; durable reply retained')
          reply = await host.requestProfile<EngineDelivery>(
            route,
            'bot.chat.deliver_status',
            { envelope_id: `reply-${envelopeId}` },
            RELAY_ENGINE_CALL_MS,
            { spawnPriority: 'foreground' }
          )
        }
      }
    } catch {
      // The sender's engine retries the durable reply on its next scan/start.
    }
  }

  if (!envelopeId) {
    return
  }

  if (!target) {
    await postReply({
      error: `connection '${envelope?.target_connection}' is not connected to this Desktop right now`
    })

    return
  }

  // Needs-attention hook (#93091 item 3): a delivered background DM is
  // this bot's "good turn"; a classified delivery failure badges it.
  const attentionKey = `${target.id}::${String(envelope?.target_profile || '')}`

  try {
    const res = await deliverToTarget(target, sender, envelope, envelopeId)

    clearBotAttention(attentionKey)
    await postReply({
      reply: String(res?.reply || '')
    })
  } catch (error: any) {
    // #93091: bot_relay.deliver classifies the failed turn and ships the
    // typed code in the JSON-RPC error's `data.reason`; forward it into
    // the sender-side reply file so the waiter (and the sending agent)
    // get the machine-readable cause, and prefer it for the badge —
    // classified codes beat free-text re-parsing.
    const reason = String(error?.data?.reason || '').trim()
    noteBotAttention(attentionKey, reason || error?.message || error)
    await postReply({
      error: String(error?.message || error || 'delivery failed'),
      ...(reason
        ? {
            reason
          }
        : {})
    })
  }
}

/** Push-notified drain (#93091): collapse a burst of pending signals into
 *  one drain call ~RELAY_PUSH_DEBOUNCE_MS after the first signal. */
function scheduleRelayPushDrain() {
  if (relay.disposed || typeof setTimeout !== 'function') {
    return
  }

  if (relay.pushDebounceTimer !== null) {
    return
  }

  relay.pushDebounceTimer = setTimeout(() => {
    relay.pushDebounceTimer = null
    void drainRelayOutboxes()
  }, RELAY_PUSH_DEBOUNCE_MS)
}

/** Stamp the heartbeat and look at the local outbox without waking any backend (the main process counts the files). */
async function localTick() {
  if (relay.disposed) {
    return
  }

  const tick = await (typeof window === 'undefined' ? undefined : window.factrDesktop?.botRelay?.tick?.())?.catch?.(
    () => null
  )

  if (tick?.ok && tick.pending > 0) {
    relay.localPending = true
    scheduleRelayPushDrain()
  }
}

export function startBotRelay() {
  relay.disposed = false
  relay.rosterClearedFor = null

  // Source-shape test harnesses evaluate plugin.js without DOM timers —
  // the relay only runs where a real event loop exists.
  if (typeof setInterval !== 'function' || typeof clearInterval !== 'function') {
    return
  }

  if (relay.rosterTimer === null) {
    relay.rosterTimer = setInterval(() => void syncRelayRosters(), RELAY_ROSTER_INTERVAL_MS)
    void syncRelayRosters()
  }

  if (relay.drainTimer === null) {
    relay.drainTimer = setInterval(() => void drainRelayOutboxes(), RELAY_DRAIN_INTERVAL_MS)
  }

  if (relay.localTimer === null) {
    relay.localTimer = setInterval(() => void localTick(), RELAY_LOCAL_TICK_MS)
    void localTick()
  }

  // Push path: the gateway change watcher broadcasts when an envelope hits
  // the outbox; drain immediately (debounced) instead of waiting the poll
  // out. Feature-detected — older shells have no host.onEvent — and the 4s
  // poll above stays untouched as the backstop either way.
  if (relay.pushUnsub === null && typeof host.onEvent === 'function') {
    relay.pushUnsub = host.onEvent('bot_relay.outbox.pending', () => scheduleRelayPushDrain())
  }
}

export function stopBotRelay() {
  relay.disposed = true
  // A rerun remembered mid-drain must not leak into the next start —
  // it would fire one stale drain after restart.
  relay.drainRerun = false
  // Queued deliveries check `disposed` before they run; forget the lane tails
  // so a restart starts every target fresh instead of behind stale chains.
  relayLanes.clear()
  // Unpin every relay-retained socket (#93594): with the relay stopped the
  // pooled entries return to dispose-at-refcount-0 semantics.
  releaseRelayRetention()

  if (relay.rosterTimer !== null) {
    clearInterval(relay.rosterTimer)
    relay.rosterTimer = null
  }

  if (relay.drainTimer !== null) {
    clearInterval(relay.drainTimer)
    relay.drainTimer = null
  }

  if (relay.localTimer !== null) {
    clearInterval(relay.localTimer)
    relay.localTimer = null
  }

  relay.localPending = false

  if (relay.pushDebounceTimer !== null) {
    clearTimeout(relay.pushDebounceTimer)
    relay.pushDebounceTimer = null
  }

  if (relay.pushUnsub !== null) {
    try {
      relay.pushUnsub()
    } catch {
      // Disposer from an older shell shape — never break teardown.
    }

    relay.pushUnsub = null
  }
}
