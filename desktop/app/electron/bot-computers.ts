// Bot computers: one hardened Docker container per bot, running that bot's own Factr-I engine.
//
// This module owns EVERY docker call for bot computers (docker CLI through execFile, args arrays only, never a shell
// string). The renderer and the bots never get the Docker socket. Design: bots-virtual-computers-design.md 3.2/3.3/6.
//
// Shared Docker Desktop rule: everything we create or remove carries the `bots-` name prefix AND the `factr.bot`
// label. reconcile() only ever lists or removes containers that match both. Nothing here prunes.

import { execFile } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

/** The run contract with the image (computer/Dockerfile). Keep every adjustable name here. */
export const BOT_COMPUTER = {
  /** Image repository; the tag is configured (a hash of engine/, backend/, computer/). */
  imageRepo: 'bots-factr-bot-computer',
  defaultTag: 'dev',
  /** Container name prefix: `<namePrefix><profile>`. */
  namePrefix: 'bots-factr-bot-',
  /** One shared bridge network with inter-container traffic off, so bots cannot reach each other. */
  network: 'bots-factr-net',
  networkOpts: ['-o', 'com.docker.network.bridge.enable_icc=false'],
  /** Marker file in the profile folder while the container exists; the Python relay refuses local delivery to it. */
  marker: '.computer',
  /**
   * Optional engine auth store (OAuth credentials live in the engine's $FACTR_HOME, not the profile folder).
   * ponytail: the image's contract for this is not published yet (computer/README.md); adjust here. Off unless
   * configureBotComputers({authStoreHostDir}) is given. Mounted read-only, so token refresh cannot write back.
   */
  authStore: { containerPath: '/data/engine-home', env: 'FACTR_HOME' },
  labelBot: 'factr.bot',
  labelVersion: 'factr.computer.v',
  version: '1',
  enginePort: 8642,
  /** Container path of the bot's profile folder. */
  mountRoot: '/data/profiles',
  envBot: 'FACTR_BOT',
  envToken: 'FACTR_ENGINE_TOKEN',
  tokenLabel: 'factr-bot:',
  limits: { pids: '2048', memory: '2g', cpus: '2', shm: '512m' },
  idleMs: 30 * 60_000,
  sweepMs: 60_000,
  stopGraceSeconds: '10'
} as const

/** Same shape as DESKTOP_PROFILE_NAME_RE (desktop-profile.ts); also a legal docker name suffix. */
const PROFILE_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/

const DOCKER_PATHS = [
  '/usr/local/bin/docker',
  '/opt/homebrew/bin/docker',
  '/Applications/Docker.app/Contents/Resources/bin/docker'
]

export type ComputerState = 'off' | 'starting' | 'on' | 'no-docker' | 'error'

export interface ComputerHandle {
  url: string
  token: string
  port: number
}

export interface ComputerStatus {
  state: ComputerState
  busy: boolean
  /** Set when state is 'no-docker'. */
  reason?: 'not-installed' | 'not-running'
  /** Set when state is 'on'. */
  memUsedBytes?: number
  memLimitBytes?: number
  /** ISO time the container started (uptime = now - startedAt). */
  startedAt?: string
  error?: string
}

export type DockerRun = (args: string[], opts?: { env?: Record<string, string>; timeoutMs?: number }) => Promise<string>

export interface BotComputerDeps {
  /** `<FACTR_CONFIG_HOME>`; profile folders live under `<configHome>/profiles/<profile>`. */
  configHome: string
  /** The app master secret; the per-bot token is derived from it. Stable across restarts. */
  masterSecret: () => string
  /** The engine health wait the pool backend start uses (main.ts waitForFactr). */
  waitReady: (baseUrl: string, token: string) => Promise<unknown>
  /** Pool cap (pool-limits.ts maxBackends). */
  maxRunning?: () => number
  /** A computer stopped on its own (idle or cap): drop its pool entry so the renderer re-dials. */
  onStopped?: (profile: string) => void
  imageTag?: string
  /** Host dir of the engine auth store to mount read-only (see BOT_COMPUTER.authStore). Omit to mount nothing. */
  authStoreHostDir?: string
  run?: DockerRun
  now?: () => number
}

interface Tracked {
  lastActive: number
  /** Reasons the idle stop must skip this bot: 'turn' (a turn or routine runs), 'view' (the screen is open). */
  holds: Set<string>
}

let deps: BotComputerDeps | null = null
let dockerBin: string | null | undefined
const tracked = new Map<string, Tracked>()
const starting = new Set<string>()
const locks = new Map<string, Promise<unknown>>()
let sweeper: NodeJS.Timeout | null = null
let availability: { at: number; value: boolean; reason?: 'not-installed' | 'not-running' } | null = null

export function configureBotComputers(next: BotComputerDeps) {
  deps = next
  availability = null
}

function cfg(): BotComputerDeps {
  if (!deps) {
    throw new Error('Bot computers are not configured.')
  }

  return deps
}

const now = () => (deps?.now ? deps.now() : Date.now())

export function validateBotProfile(profile: unknown): string {
  if (typeof profile !== 'string' || !PROFILE_RE.test(profile) || profile === 'default') {
    throw new Error(`Invalid bot profile name: ${JSON.stringify(profile)}`)
  }

  return profile
}

export interface BotRouteFacts {
  /** The packaged Rust engine is primary: one profile per process (ProfileRouteOptions.primarySingleProfile). */
  primarySingleProfile?: boolean
  primaryProfile?: null | string
  globalRemote?: boolean
  profileRemoteOverride?: boolean
  primaryRemoteActive?: boolean
  /** FACTR_BOT_COMPUTERS=0 turns computers off (per-profile host engines instead). */
  disabled?: boolean
}

/**
 * Which backend a LOCAL non-primary profile gets once the router sent it to the pool (case 6 refuses to share a
 * single-profile primary): its own computer when Docker is up, else its own host engine. Never both.
 * `null`: not a bot-computer case (primary profile, remote, dev Python gateway), the caller keeps its normal path.
 */
export function chooseBotBackend(
  profile: string,
  facts: BotRouteFacts,
  dockerUp: boolean
): 'container' | 'host-engine' | null {
  const isBot =
    facts.primarySingleProfile &&
    !facts.disabled &&
    !facts.globalRemote &&
    !facts.profileRemoteOverride &&
    !facts.primaryRemoteActive &&
    profile !== (facts.primaryProfile || 'default') &&
    PROFILE_RE.test(profile) &&
    profile !== 'default'

  if (!isBot) {
    return null
  }

  return dockerUp ? 'container' : 'host-engine'
}

export const computerName = (profile: string) => `${BOT_COMPUTER.namePrefix}${validateBotProfile(profile)}`

export const computerImage = () => `${BOT_COMPUTER.imageRepo}:${deps?.imageTag || BOT_COMPUTER.defaultTag}`

/** HMAC-SHA256(master secret, 'factr-bot:' + profile) as 64 hex chars (the engine needs a token of 32+ chars). */
export function deriveBotToken(profile: string, secret: string = cfg().masterSecret()): string {
  return crypto
    .createHmac('sha256', secret)
    .update(BOT_COMPUTER.tokenLabel + validateBotProfile(profile))
    .digest('hex')
}

function findDockerBinary(): string | null {
  if (dockerBin !== undefined) {
    return dockerBin
  }

  const exe = process.platform === 'win32' ? 'docker.exe' : 'docker'

  const candidates = [
    process.env.FACTR_DOCKER_BINARY,
    ...(process.env.PATH || '').split(path.delimiter).map(dir => dir && path.join(dir, exe)),
    ...DOCKER_PATHS
  ]

  dockerBin = candidates.find(candidate => candidate && fs.existsSync(candidate)) || null

  return dockerBin
}

const defaultRun: DockerRun = (args, opts = {}) =>
  new Promise((resolve, reject) => {
    const bin = findDockerBinary()

    if (!bin) {
      reject(Object.assign(new Error('Docker is not installed.'), { code: 'ENOENT' }))

      return
    }

    execFile(
      bin,
      args,
      {
        env: { ...process.env, ...opts.env },
        timeout: opts.timeoutMs ?? 60_000,
        maxBuffer: 4 << 20,
        windowsHide: true
      },
      (error, stdout, stderr) =>
        error ? reject(Object.assign(error, { stderr: String(stderr || '') })) : resolve(String(stdout))
    )
  })

const docker: DockerRun = (args, opts) => (deps?.run || defaultRun)(args, opts)

async function probeDocker() {
  if (availability && now() - availability.at < 5_000) {
    return availability
  }

  let value = false
  let reason: 'not-installed' | 'not-running' | undefined

  try {
    await docker(['version', '--format', '{{.Server.Version}}'], { timeoutMs: 5_000 })
    value = true
  } catch (error: any) {
    reason = error?.code === 'ENOENT' ? 'not-installed' : 'not-running'
  }

  availability = { at: now(), value, reason }

  return availability
}

export async function dockerAvailable(): Promise<boolean> {
  return (await probeDocker()).value
}

// One operation per bot at a time (create/start/stop/destroy never interleave).
function serialized<T>(profile: string, fn: () => Promise<T>): Promise<T> {
  const next = (locks.get(profile) || Promise.resolve()).catch(() => {}).then(fn)
  locks.set(profile, next)
  void next.finally(() => locks.get(profile) === next && locks.delete(profile)).catch(() => {})

  return next
}

interface Inspect {
  status: string
  image: string
  botLabel: string
  startedAt: string
}

async function inspect(name: string): Promise<Inspect | null> {
  try {
    const out = await docker([
      'inspect',
      '--format',
      `{{.State.Status}}|{{.Config.Image}}|{{index .Config.Labels "${BOT_COMPUTER.labelBot}"}}|{{.State.StartedAt}}`,
      name
    ])

    const [status = '', image = '', botLabel = '', startedAt = ''] = out.trim().split('|')

    return { status, image, botLabel, startedAt }
  } catch (error: any) {
    if (/no such (object|container)/i.test(`${error?.stderr || ''}${error?.message || ''}`)) {
      return null
    }

    throw error
  }
}

const hostProfileDir = (profile: string) => path.join(cfg().configHome, 'profiles', profile)

async function ensureNetwork() {
  try {
    await docker(['network', 'inspect', BOT_COMPUTER.network])
  } catch {
    await docker(['network', 'create', ...BOT_COMPUTER.networkOpts, BOT_COMPUTER.network])
  }
}

/** The exact `docker create` argv. Exported for tests. */
export function createArgs(profile: string, image: string, hostDir: string, authStoreHostDir?: string): string[] {
  const name = computerName(profile)
  const L = BOT_COMPUTER.limits

  return [
    'create',
    '--name', name,
    '--label', `${BOT_COMPUTER.labelBot}=${profile}`,
    '--label', `${BOT_COMPUTER.labelVersion}=${BOT_COMPUTER.version}`,
    '--network', BOT_COMPUTER.network,
    // The bot reaches host services (a local model server) at host.docker.internal on every platform.
    '--add-host', 'host.docker.internal:host-gateway',
    '--cap-drop', 'ALL',
    '--security-opt', 'no-new-privileges',
    '--pids-limit', L.pids,
    '--memory', L.memory,
    '--cpus', L.cpus,
    '--shm-size', L.shm,
    // Loopback only: the bot's engine is reachable from this machine, never the LAN.
    '-p', `127.0.0.1:0:${BOT_COMPUTER.enginePort}`,
    '-v', `${hostDir}:${BOT_COMPUTER.mountRoot}/${profile}`,
    ...(authStoreHostDir
      ? [
          '-v', `${authStoreHostDir}:${BOT_COMPUTER.authStore.containerPath}:ro`,
          '-e', `${BOT_COMPUTER.authStore.env}=${BOT_COMPUTER.authStore.containerPath}`
        ]
      : []),
    '-e', `${BOT_COMPUTER.envBot}=${profile}`,
    // Name only: the value rides the docker client's env, so it is not in argv (`ps`).
    '-e', BOT_COMPUTER.envToken,
    // ponytail: a model provider on localhost (Ollama, LM Studio) must still be rewritten to host.docker.internal
    // in the bot's config (the --add-host above only makes the name resolve). Not done in the MVP.
    // ponytail: Linux hosts need --user <host uid> for the bind mount; Docker Desktop (Mac/Windows) maps ownership.
    image
  ]
}

const markerPath = (profile: string) => path.join(hostProfileDir(profile), BOT_COMPUTER.marker)

function writeMarker(profile: string) {
  fs.writeFileSync(markerPath(profile), JSON.stringify({ container: computerName(profile), v: BOT_COMPUTER.version }))
}

function parseHostPort(out: string): number {
  const lines = out.split('\n').map(l => l.trim()).filter(Boolean)
  const line = lines.find(l => l.startsWith('127.0.0.1:')) || lines[0] || ''
  const port = Number(line.match(/:(\d+)$/)?.[1])

  if (!Number.isInteger(port) || port <= 0) {
    throw new Error(`Could not read the published port for the bot computer (${JSON.stringify(out.trim())}).`)
  }

  return port
}

function track(profile: string) {
  const entry = tracked.get(profile) || { lastActive: now(), holds: new Set<string>() }
  entry.lastActive = now()
  tracked.set(profile, entry)

  if (!sweeper) {
    sweeper = setInterval(() => void sweepIdle().catch(() => {}), BOT_COMPUTER.sweepMs)
    sweeper.unref?.()
  }
}

export function touchComputer(profile: string) {
  const entry = tracked.get(profile)

  if (entry) {
    entry.lastActive = now()
  }
}

/** Mark a bot as working ('turn') or being watched ('view'): the idle stop and the cap skip it. */
export function setComputerHold(profile: string, reason: string, on: boolean) {
  const entry = tracked.get(validateBotProfile(profile))

  if (entry) {
    entry.holds[on ? 'add' : 'delete'](reason)
    entry.lastActive = now()
  }
}

async function stopForCap(except: string) {
  const max = deps?.maxRunning?.() ?? 3
  const running = [...tracked.entries()].filter(([p]) => p !== except)

  if (running.length < max) {
    return
  }

  const idle = running.filter(([, t]) => t.holds.size === 0).sort((a, b) => a[1].lastActive - b[1].lastActive)
  // Every running computer is busy: exceed the soft cap rather than kill a live session.
  const victim = idle[0]?.[0]

  if (victim) {
    await stopComputer(victim)
  }
}

export async function ensureComputer(profile: string): Promise<ComputerHandle> {
  validateBotProfile(profile)

  return serialized(profile, async () => {
    cfg()

    if (!(await dockerAvailable())) {
      throw new Error('Docker is not available, so this bot cannot get its own computer.')
    }

    const name = computerName(profile)
    const image = computerImage()
    const token = deriveBotToken(profile)
    starting.add(profile)

    try {
      const info = await inspect(name)

      if (info && info.botLabel !== profile) {
        throw new Error(`A container named ${name} exists but does not belong to this bot; refusing to use it.`)
      }

      if (!info) {
        const hostDir = hostProfileDir(profile)

        // Never recreate a deleted profile folder: a missing folder means the bot is gone.
        if (!fs.existsSync(hostDir)) {
          throw new Error(`Profile folder for "${profile}" does not exist.`)
        }

        await ensureNetwork()
        await docker(createArgs(profile, image, hostDir, cfg().authStoreHostDir), {
          env: { [BOT_COMPUTER.envToken]: token }
        })
      }

      // The Python relay refuses host-side delivery into a bot whose container exists (second SQLite writer).
      writeMarker(profile)

      if (info?.status !== 'running') {
        await stopForCap(profile)
        await docker(['start', name])
      }

      const port = parseHostPort(await docker(['port', name, `${BOT_COMPUTER.enginePort}/tcp`]))
      const url = `http://127.0.0.1:${port}`
      await cfg().waitReady(url, token)
      track(profile)

      return { url, token, port }
    } finally {
      starting.delete(profile)
    }
  })
}

export async function stopComputer(profile: string): Promise<void> {
  validateBotProfile(profile)

  return serialized(profile, async () => {
    tracked.delete(profile)

    try {
      await docker(['stop', '-t', BOT_COMPUTER.stopGraceSeconds, computerName(profile)], { timeoutMs: 30_000 })
    } catch (error: any) {
      // No docker, or no such container: already off.
      if (error?.code !== 'ENOENT' && !/no such (object|container)/i.test(`${error?.stderr || ''}`)) {
        throw error
      }
    }

    deps?.onStopped?.(profile)
  })
}

export async function destroyComputer(profile: string): Promise<void> {
  validateBotProfile(profile)

  return serialized(profile, async () => {
    const name = computerName(profile)
    tracked.delete(profile)
    fs.rmSync(markerPath(profile), { force: true })

    try {
      await docker(['rm', '-f', name], { timeoutMs: 30_000 })
    } catch (error: any) {
      if (error?.code !== 'ENOENT' && !/no such (object|container)/i.test(`${error?.stderr || ''}`)) {
        throw error
      }
    }
  })
}

function parseBytes(text: string): number {
  const m = text.trim().match(/^([\d.]+)\s*([KMGT]?i?B)$/i)

  if (!m) {
    return 0
  }

  const unit = m[2].toUpperCase()
  const pow = { B: 0, KB: 1, KIB: 1, MB: 2, MIB: 2, GB: 3, GIB: 3, TB: 4, TIB: 4 }[unit] ?? 0
  const base = unit.includes('I') ? 1024 : 1000

  return Math.round(Number(m[1]) * base ** pow)
}

export async function computerStatus(profile: string): Promise<ComputerStatus> {
  validateBotProfile(profile)
  const busy = (tracked.get(profile)?.holds.size ?? 0) > 0
  const probe = await probeDocker()

  if (!probe.value) {
    return { state: 'no-docker', busy, reason: probe.reason }
  }

  try {
    const info = await inspect(computerName(profile))

    if (!info || info.status !== 'running') {
      return { state: starting.has(profile) ? 'starting' : 'off', busy }
    }

    if (starting.has(profile)) {
      return { state: 'starting', busy }
    }

    const out: ComputerStatus = { state: 'on', busy, startedAt: info.startedAt }

    try {
      const stats = await docker(['stats', '--no-stream', '--format', '{{.MemUsage}}', computerName(profile)], {
        timeoutMs: 10_000
      })

      const [used = '', limit = ''] = stats.trim().split('/')
      out.memUsedBytes = parseBytes(used)
      out.memLimitBytes = parseBytes(limit)
    } catch {
      // Memory is a nicety; the computer is on regardless.
    }

    return out
  } catch (error: any) {
    return { state: 'error', busy, error: String(error?.stderr || error?.message || error).slice(0, 300) }
  }
}

/**
 * App start: remove containers whose bot is gone, and adopt the running ones into the idle timer.
 * ONLY containers with the `factr.bot` label AND the `bots-` name prefix are ever considered.
 */
export async function reconcileComputers(existingProfiles: Iterable<string>): Promise<string[]> {
  if (!(await dockerAvailable())) {
    return []
  }

  const keep = new Set(existingProfiles)
  const removed: string[] = []

  const out = await docker([
    'ps',
    '-a',
    '--filter',
    `label=${BOT_COMPUTER.labelBot}`,
    '--filter',
    `name=${BOT_COMPUTER.namePrefix}`,
    '--format',
    `{{.Names}}|{{.State}}|{{index .Labels "${BOT_COMPUTER.labelBot}"}}`
  ])

  const lines = out.split('\n').filter(Boolean)

  for (const line of lines) {
    const [name = '', state = '', profile = ''] = line.trim().split('|')

    // Docker's name filter is a substring match; re-check strictly before touching anything.
    if (!profile || !PROFILE_RE.test(profile) || name !== `${BOT_COMPUTER.namePrefix}${profile}`) {
      continue
    }

    if (keep.has(profile)) {
      if (state === 'running') {
        track(profile)
      }

      continue
    }

    await destroyComputer(profile)
    removed.push(profile)
  }

  // The shared network is ours too: drop it once the last bot computer is gone (docker refuses while one is attached).
  if (removed.length > 0 && removed.length === lines.length) {
    await docker(['network', 'rm', BOT_COMPUTER.network]).catch(() => {})
  }

  return removed
}

async function sweepIdle() {
  const cutoff = now() - BOT_COMPUTER.idleMs

  for (const [profile, t] of [...tracked]) {
    if (t.holds.size === 0 && t.lastActive < cutoff) {
      await stopComputer(profile).catch(() => {})
    }
  }

  if (tracked.size === 0 && sweeper) {
    clearInterval(sweeper)
    sweeper = null
  }
}

/** Test hook: forget all module state. */
export function resetBotComputersForTest() {
  deps = null
  dockerBin = undefined
  availability = null
  tracked.clear()
  starting.clear()
  locks.clear()

  if (sweeper) {
    clearInterval(sweeper)
    sweeper = null
  }
}

export { sweepIdle as sweepIdleComputers }
