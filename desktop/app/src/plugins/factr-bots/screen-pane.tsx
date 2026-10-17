/**
 * Bot Screen pane — live view of a bot's headless desktop with Take over / Hand back.
 *
 * State authority: the BACKEND owns runtime + lease (`display.status`, pushed
 * as `display.lease` events); this pane paints a cache of it. The RFB stream
 * is a sibling WebSocket handed to noVNC's RFB; `viewOnly` here is UX only —
 * the gateway drops input from anyone but the lease holder.
 *
 * Every attach spends a single-use ticket, so a lease flip that the bridge
 * answers with close 4000 (`control-taken`) re-attaches in watch mode on a
 * fresh ticket under a brief caption; a bridge that keeps evicting fresh
 * attaches is bounded to a few rapid retries before the error state.
 */

import { Button, Codicon, EmptyState, GlyphSpinner, host, Tip, useValue } from '@factr/plugin-sdk'
import type { RpcEvent } from '@factr/plugin-sdk'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'

import { type BotComputer, formatMemory, formatUptime, useBotComputer, useComputerViewHold } from './computer'
import { useBots } from './i18n'
import {
  type DisplayLease,
  type DisplayObserveResult,
  displayRequest,
  type DisplayStatus,
  isDisplayUnavailable,
  isEventForBotScreen,
  leaseHeldBy,
  resolveScreenWsUrl,
  retainBotScreen,
  viewerHash
} from './screen-connection'
import { ScreenInstallCard } from './screen-install'
import {
  $screenState,
  beginScreenStatusRequest,
  screenStateFor,
  setScreenLease,
  setScreenStatus,
  setScreenUnavailable,
  setScreenViewer
} from './screen-state'
import type { RosterRow } from './types'

type RfbLike = {
  viewOnly: boolean
  scaleViewport: boolean
  resizeSession: boolean
  focusOnClick: boolean
  background: string
  qualityLevel: number
  addEventListener: (type: string, handler: (event: { detail?: { clean?: boolean; reason?: string } }) => void) => void
  disconnect: () => void
  focus: () => void
}

type ConnState = 'idle' | 'attaching' | 'live' | 'error'

/** Bridge close code when another viewer took the lease (mirrors tui_gateway display bridge). */
const CLOSE_CONTROL_TAKEN = 4000
/** Evictions arriving this soon after dialing count toward the loop budget; slower ones reset it. */
const EVICTION_LOOP_WINDOW_MS = 10_000
const MAX_RAPID_EVICTIONS = 3

async function loadRfb(): Promise<
  new (target: HTMLElement, socket: WebSocket, options?: Record<string, unknown>) => RfbLike
> {
  const mod = (await import('@novnc/novnc')) as unknown as { default: new (...args: never[]) => RfbLike }

  return mod.default as unknown as new (
    target: HTMLElement,
    socket: WebSocket,
    options?: Record<string, unknown>
  ) => RfbLike
}

export function BotScreenPane({ bot }: { bot: RosterRow }) {
  const t = useBots()
  const screen = useValue($screenState)
  const state = screenStateFor(screen, bot)
  const status = state?.status ?? null
  const lease = state?.lease ?? status?.lease ?? null
  // The server mints this window's viewer id per attach (`display.observe`); the lease
  // names its holder by hash, so a reload can never inherit a stale holder's authority.
  const viewer = state?.viewer ?? null
  const iHold = leaseHeldBy(lease, viewer)

  const canvasHost = useRef<HTMLDivElement | null>(null)
  const rfb = useRef<RfbLike | null>(null)
  const socket = useRef<WebSocket | null>(null)
  // Pins the bot's pooled gateway socket for the attach lifetime so display.lease
  // events keep arriving for an inactive registry-routed bot.
  const retention = useRef<(() => void) | null>(null)
  const [conn, setConn] = useState<ConnState>('idle')
  const [error, setError] = useState<null | string>(null)
  const [busy, setBusy] = useState(false)
  // Caption after a 4000 eviction; cleared once the watch-mode re-attach paints live frames.
  const [evicted, setEvicted] = useState(false)
  const attachGeneration = useRef(0)
  const dialedAt = useRef(0)
  const rapidEvictions = useRef(0)

  const refresh = useCallback(async () => {
    // A reply that a newer request, a start result or a pushed event overtook is dropped by
    // the store; otherwise a slow `running: true` from before a stop repaints a dead screen.
    const request = beginScreenStatusRequest(bot)

    try {
      const next = await displayRequest<DisplayStatus>(bot, 'display.status')
      setScreenStatus(bot, next, request)
      setError(null)
    } catch (err) {
      if (isDisplayUnavailable(err)) {
        setScreenUnavailable(bot)
      }

      setError(err instanceof Error ? err.message : String(err))
    }
  }, [bot])

  useEffect(() => {
    void refresh()

    const offLease = host.onEvent('display.lease', (event: RpcEvent) => {
      const payload = event.payload as { lease?: DisplayLease } | undefined

      if (payload?.lease && isEventForBotScreen(bot, event, status?.profile_key)) {
        setScreenLease(bot, payload.lease)
      }
    })

    // A start/stop made outside this window (CLI, gateway auto-start, another Desktop) is
    // pushed by the serve-side runtime watcher. A stopped pane with no sibling portal or
    // hero mounted for this bot has no other way to learn of it.
    const offStatus = host.onEvent('display.status', (event: RpcEvent) => {
      const payload = event.payload as DisplayStatus | undefined

      if (payload?.profile_key && isEventForBotScreen(bot, event, status?.profile_key)) {
        setScreenStatus(bot, payload)
      }
    })

    return () => {
      offLease()
      offStatus()
    }
  }, [bot, refresh, status?.profile_key])

  const computer = useBotComputer(bot)
  useComputerViewHold(bot)

  // The computer starting, stopping or dying changes which engine answers `display.*`: re-ask.
  useEffect(() => {
    if (computer) {
      void refresh()
    }
  }, [computer?.state, refresh]) // eslint-disable-line react-hooks/exhaustive-deps

  const startComputer = useCallback(async () => {
    await computer?.start()
    setConn('idle')
    void refresh()
  }, [computer, refresh])

  const detach = useCallback((handBack = false) => {
    attachGeneration.current += 1

    // noVNC closes without a status. Intentional pane closure must send 1000
    // first; reconnect teardown must keep the human lease instead.
    if (handBack) {
      socket.current?.close(1000)
    }

    rfb.current?.disconnect()
    rfb.current = null
    socket.current?.close()
    socket.current = null
    retention.current?.()
    retention.current = null
  }, [])

  const attach = useCallback(
    async (auto = false) => {
      if (!canvasHost.current) {
        return
      }

      if (!auto) {
        rapidEvictions.current = 0
      }

      detach()
      const generation = attachGeneration.current
      setConn('attaching')
      setError(null)

      try {
        // Load the client BEFORE dialing: noVNC's Websock installs its own `onopen`, so a socket that
        // opened while the dynamic import was still in flight never hands it the open event.
        const Rfb = await loadRfb()
        const retain = await retainBotScreen(bot)

        if (generation !== attachGeneration.current) {
          retain()

          return
        }

        retention.current = retain
        // Re-present the id this window already minted: a reconnect (network blip, 4000 eviction, Reconnect
        // button) must keep the human's lease bound to THIS pane, or the new stream is watch-only and the
        // old lease can only be cleared by force. The server honours a minted id only on the connection
        // that minted it; a foreign or stale id is silently replaced.
        const priorViewer = screenStateFor($screenState.get(), bot)?.viewer?.id

        const observe = await displayRequest<DisplayObserveResult>(
          bot,
          'display.observe',
          priorViewer ? { viewer_id: priorViewer } : {}
        )

        const minted = { id: observe.viewer_id, hash: await viewerHash(observe.viewer_id) }
        setScreenStatus(bot, observe)
        const url = await resolveScreenWsUrl(bot, observe.ticket)

        if (generation !== attachGeneration.current || !canvasHost.current) {
          return
        }

        setScreenViewer(bot, minted)
        dialedAt.current = Date.now()
        const ws = new WebSocket(url)
        ws.binaryType = 'arraybuffer'
        socket.current = ws
        // noVNC 1.7's `disconnect` detail carries only {clean}; the bridge's verdict lives in
        // the raw close frame (4000 = control-taken). Listen here, before RFB installs its
        // own `onclose`, so the code is known by the time the disconnect event fires.
        let closeCode = 0
        ws.addEventListener('close', event => {
          closeCode = event.code
        })
        const client = new Rfb(canvasHost.current, ws, { shared: true })
        client.scaleViewport = true
        client.resizeSession = false
        client.focusOnClick = true
        client.background = 'transparent'
        client.qualityLevel = 7
        client.viewOnly = !leaseHeldBy(observe.lease, minted)
        client.addEventListener('connect', () => {
          if (generation === attachGeneration.current) {
            setConn('live')
            setEvicted(false)
          }
        })
        client.addEventListener('disconnect', event => {
          // noVNC logs "Tried changing state of a disconnected RFB object" if we later call
          // disconnect() on a client that already closed itself (eviction, stream loss).
          if (rfb.current === client) {
            rfb.current = null
          }

          if (generation !== attachGeneration.current) {
            return
          }

          const reason = event.detail?.reason ?? ''

          if (closeCode === CLOSE_CONTROL_TAKEN || reason.includes('control-taken')) {
            // Evicted: the socket is dead, so the frozen frame must not stay up. Going idle
            // re-attaches in watch mode on a fresh ticket; a bridge that evicts every fresh
            // attach within the window is bounded, then lands in the error state + Reconnect.
            rapidEvictions.current =
              Date.now() - dialedAt.current < EVICTION_LOOP_WINDOW_MS ? rapidEvictions.current + 1 : 1

            if (rapidEvictions.current > MAX_RAPID_EVICTIONS) {
              setEvicted(false)
              setConn('error')
              setError(t.screen.controlTaken)
            } else {
              setEvicted(true)
              setConn('idle')
            }
          } else if (event.detail?.clean) {
            setConn('idle')
          } else {
            setConn('error')
            setError(reason || t.screen.streamLost)
          }

          void refresh()
        })
        rfb.current = client
      } catch (err) {
        if (generation === attachGeneration.current) {
          setConn('error')
          setError(err instanceof Error ? err.message : String(err))
        }
      }
    },
    [bot, detach, refresh, t.screen.controlTaken, t.screen.streamLost]
  )

  // Visibility is not lifecycle: the stream stays attached while the pane is
  // hidden; only unmount tears it down (and hands control back server-side).
  useEffect(() => () => detach(true), [detach])

  useEffect(() => {
    if (status?.running && conn === 'idle') {
      void attach(true)
    }
  }, [attach, conn, status?.running])

  useEffect(() => {
    if (rfb.current) {
      rfb.current.viewOnly = !iHold

      if (iHold) {
        rfb.current.focus()
      }
    }
  }, [iHold])

  const start = useCallback(async () => {
    setBusy(true)

    try {
      const next = await displayRequest<DisplayStatus>(bot, 'display.start')
      setScreenStatus(bot, next)
      setConn('idle')
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }, [bot])

  const takeOver = useCallback(async () => {
    // The button is disabled without a viewer; the guard keeps a keyboard-activated
    // stale closure from sending an empty viewer_id the server rejects.
    if (!viewer) {
      return
    }

    setBusy(true)

    try {
      const result = await displayRequest<{ lease: DisplayLease }>(bot, 'display.lease.acquire', {
        viewer_id: viewer.id
      })

      setScreenLease(bot, result.lease)

      if (conn !== 'live') {
        void attach()
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }, [attach, bot, conn, viewer])

  // `force` is the escape hatch for a lease this window no longer owns (a reload
  // minted a fresh viewer id; the old one still holds): the server refuses a
  // plain release from anyone but the holder.
  const handBack = useCallback(
    async (force = false) => {
      setBusy(true)

      try {
        const params = force ? { force: true } : { viewer_id: viewer?.id }
        const result = await displayRequest<{ lease: DisplayLease }>(bot, 'display.lease.release', params)
        setScreenLease(bot, result.lease)
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
      }
    },
    [bot, viewer?.id]
  )

  // A bot whose computer is not up has no desktop to show: say why, with the one action.
  // Also when the display reads "not running" while the computer itself is off or failed: that is one problem
  // (Start computer), not two buttons and a stale "backend retired" error from a bot engine that is gone.
  if (
    computer &&
    computer.state !== 'on' &&
    (computer.state === 'starting' ||
      state?.unavailable ||
      (status && (!status.supported || (!status.running && computer.state !== 'no-docker'))))
  ) {
    return <ComputerGate computer={computer} onStart={() => void startComputer()} />
  }

  if (state?.unavailable) {
    return <EmptyState description={t.screen.portalUnavailable} title={t.screen.unavailableTitle} />
  }

  if (status && !status.supported) {
    return <EmptyState description={t.screen.unsupportedBody} title={t.screen.unsupportedTitle} />
  }

  if (status && !status.installed) {
    return <ScreenInstallCard bot={bot} onInstalled={next => setScreenStatus(bot, next)} status={status} />
  }

  if (status && !status.running) {
    const on = computer?.state === 'on'

    // With the computer on, the line under the title is its uptime and memory, and Stop sits left of Start screen.
    return (
      <PaneGate
        actions={
          <>
            {on && computer ? <StopButton computer={computer} /> : null}
            <Button disabled={busy || Boolean(status.blocker)} onClick={() => void start()} size="sm">
              {busy ? <GlyphSpinner /> : <Codicon name="player-play-filled" size="0.875rem" />}
              {t.screen.start}
            </Button>
          </>
        }
        line={on && computer ? <OnText computer={computer} /> : (status.blocker ?? t.screen.stoppedBody)}
        title={t.screen.stoppedTitle}
      >
        {computer && !on ? <ComputerLine computer={computer} onStart={() => void startComputer()} /> : null}
        {error ? <div className="text-xs text-red-500">{error}</div> : null}
      </PaneGate>
    )
  }

  const humanOther = lease?.holder === 'human' && !iHold

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b px-3 py-1.5 text-xs">
        <Codicon name="device-desktop" />
        <span className="font-medium">{t.screen.title}</span>
        {status?.display ? (
          <span className="text-muted-foreground">
            {status.display} · {status.geometry}
          </span>
        ) : null}
        {computer ? <ComputerLine computer={computer} onStart={() => void startComputer()} /> : null}
        <span className="grow" />
        {(lease?.holder === 'human' ? lease.reason : lease?.request) ? (
          // The bot's ask (request_takeover) while it waits, and why control was taken once the human acts.
          <span
            className="max-w-[40%] truncate rounded bg-amber-500/15 px-2 py-0.5 text-amber-600 dark:text-amber-400"
            title={lease?.holder === 'human' ? lease.reason : lease?.request}
          >
            <Codicon name="bell" /> {lease?.holder === 'human' ? lease.reason : lease?.request}
          </span>
        ) : null}
        {iHold ? (
          <span className="rounded bg-red-500/15 px-2 py-0.5 font-medium text-red-600 dark:text-red-400">
            {t.screen.youControl}
          </span>
        ) : humanOther ? (
          <span className="rounded bg-muted px-2 py-0.5 text-muted-foreground">{t.screen.otherControls}</span>
        ) : (
          <span className="rounded bg-muted px-2 py-0.5 text-muted-foreground">{t.screen.agentControls}</span>
        )}
        {iHold ? (
          <Button disabled={busy} onClick={() => void handBack()} size="sm" variant="secondary">
            <Codicon name="debug-continue" /> {t.screen.handBack}
          </Button>
        ) : (
          <>
            {humanOther ? (
              <Tip label={t.screen.handBackForceHint}>
                <Button disabled={busy} onClick={() => void handBack(true)} size="sm" variant="secondary">
                  <Codicon name="debug-continue" /> {t.screen.handBackForce}
                </Button>
              </Tip>
            ) : null}
            {/* The lease is granted to a server-minted viewer id; until `display.observe` has
                minted one for this attach a Take over could only send an empty id and dead-end
                on "viewer_id required". Reconnect is the way to mint one. */}
            <Button disabled={busy || conn === 'attaching' || !viewer} onClick={() => void takeOver()} size="sm">
              <Codicon name="record-keys" /> {t.screen.takeOver}
            </Button>
          </>
        )}
        <Tip label={t.screen.reconnect}>
          <Button
            aria-label={t.screen.reconnect}
            disabled={conn === 'attaching'}
            onClick={() => void attach()}
            size="sm"
            variant="ghost"
          >
            <Codicon name="refresh" />
          </Button>
        </Tip>
      </div>
      <div
        className={
          iHold ? 'relative min-h-0 grow bg-black ring-2 ring-inset ring-red-500/70' : 'relative min-h-0 grow bg-black'
        }
      >
        {/* data-terminal: the same keyboard-ownership marker the terminal pane uses, so the app's
            type-to-focus / bare-key shortcuts never steal keystrokes meant for the remote screen.
            data-remote-screen: tells the ⌘W close-tab router this is NOT a local terminal tab —
            the chord belongs to the remote desktop, nothing local should close. */}
        <div className="absolute inset-0" data-remote-screen="" data-terminal="" ref={canvasHost} />
        {conn === 'attaching' ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-xs text-white/70">
            <GlyphSpinner /> {t.screen.attaching}
          </div>
        ) : null}
        {evicted ? (
          <div className="pointer-events-none absolute inset-x-0 top-0 bg-amber-950/80 px-3 py-1.5 text-center text-xs text-amber-100">
            {t.screen.controlTaken}
          </div>
        ) : null}
        {conn === 'error' && error ? (
          <div className="absolute inset-x-0 bottom-0 bg-red-950/80 px-3 py-1.5 text-xs text-red-200">{error}</div>
        ) : null}
      </div>
    </div>
  )
}

/** "On for 12 min, 640 MB of 2 GB". */
function OnText({ computer }: { computer: BotComputer }) {
  const { memLimitBytes, memUsedBytes, startedAt } = computer.status

  return (
    <>
      {useBots().computer.onLine(
        formatUptime(startedAt),
        memUsedBytes ? formatMemory(memUsedBytes) : null,
        memLimitBytes ? formatMemory(memLimitBytes) : null
      )}
    </>
  )
}

function StopButton({ computer }: { computer: BotComputer }) {
  const t = useBots().computer
  const [busy, setBusy] = useState(false)

  const stop = async () => {
    setBusy(true)

    try {
      await computer.stop()
    } finally {
      setBusy(false)
    }
  }

  return (
    <Button disabled={busy} onClick={() => void stop()} size="sm" variant="ghost">
      {busy ? <GlyphSpinner /> : <Codicon name="player-stop-filled" size="0.875rem" />}
      {t.stop}
    </Button>
  )
}

/** One quiet line about the computer with its Start or Stop action (the live header). */
function ComputerLine({ computer, onStart }: { computer: BotComputer; onStart: () => void }) {
  const t = useBots().computer

  if (computer.state === 'on') {
    return (
      <>
        <span className="whitespace-nowrap text-(--ui-text-secondary)">
          <OnText computer={computer} />
        </span>
        <StopButton computer={computer} />
      </>
    )
  }

  if (computer.state === 'starting') {
    return (
      <span className="inline-flex items-center gap-2 text-(--ui-text-secondary)">
        <GlyphSpinner /> {t.starting}
      </span>
    )
  }

  if (computer.state === 'no-docker') {
    return null
  }

  return (
    <>
      <span className="text-(--ui-text-secondary)">{computer.state === 'error' ? t.failedTitle : t.offTitle}</span>
      <Button onClick={onStart} size="sm" variant="ghost">
        <Codicon name="player-play-filled" size="0.875rem" />
        {t.start}
      </Button>
    </>
  )
}

const DOCKER_URL = 'https://www.docker.com/products/docker-desktop/'

/** The pane when there is no running computer to show: why, and the one thing to do. */
function ComputerGate({ computer, onStart }: { computer: BotComputer; onStart: () => void }) {
  const t = useBots().computer
  const { error, reason } = computer.status
  const mac = typeof navigator !== 'undefined' && /mac/i.test(navigator.platform)
  const open = (url: string) => void window.factrDesktop?.openExternal?.(url)
  const openDocker = () => void window.factrDesktop?.botComputer?.openDocker?.()

  const view = (() => {
    if (computer.state === 'starting') {
      return { body: t.startingBody, title: t.starting }
    }

    if (computer.state === 'no-docker') {
      return reason === 'not-installed'
        ? { action: { label: t.getDocker, run: () => open(DOCKER_URL) }, body: t.dockerNotInstalledBody, title: t.dockerNotInstalled }
        : {
            action: mac
              ? { label: t.openDocker, run: openDocker }
              : { label: t.getDocker, run: () => open(DOCKER_URL) },
            body: t.dockerNotRunningBody,
            title: t.dockerNotRunning
          }
    }

    return {
      action: { label: t.start, run: onStart },
      body: computer.state === 'error' ? error : t.offBody,
      title: computer.state === 'error' ? t.failedTitle : t.offTitle
    }
  })()

  return (
    <PaneGate
      actions={
        'action' in view && view.action ? (
          <Button onClick={view.action.run} size="sm">
            {view.action.label}
          </Button>
        ) : null
      }
      line={view.body}
      spinner={computer.state === 'starting'}
      title={view.title}
    />
  )
}

/** The one centred block every "nothing to show yet" state uses: title, a quiet line, then the actions in one row. */
function PaneGate({
  actions,
  children,
  line,
  spinner,
  title
}: {
  actions?: ReactNode
  children?: ReactNode
  line?: ReactNode
  spinner?: boolean
  title: ReactNode
}) {
  return (
    <div className="grid min-h-48 place-items-center p-6 text-center">
      <div className="flex flex-col items-center gap-2">
        <div className="inline-flex items-center gap-2 text-[length:var(--conversation-text-font-size)] leading-5 font-medium">
          {spinner ? <GlyphSpinner className="text-[0.875rem]" /> : null}
          {title}
        </div>
        {line ? (
          <div className="max-w-80 text-[0.8125rem] leading-[1.125rem] text-(--ui-text-secondary)">{line}</div>
        ) : null}
        {actions ? <div className="flex items-center gap-2">{actions}</div> : null}
        {children}
      </div>
    </div>
  )
}
