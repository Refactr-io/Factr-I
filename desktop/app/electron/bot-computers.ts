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
import net from 'node:net'
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
   * The bot's engine home (sessions, factr.db, memory, skills AND the auth store) is `/data/profiles/<bot>/engine`
   * inside the profile mount, so it persists with no extra mount. Credentials are seeded into it once from the host
   * engine home (seedBotAuth). Never mount the real engine home: it holds every session, and a read-only mount stops
   * the engine at start.
   */
  engineDir: 'engine',
  authFiles: ['auth.json', 'openai-auth.json', 'google_oauth.json', 'google_credentials.json'],
  /** How long an image-present check is trusted before asking docker again. */
  imageCheckMs: 30_000,
  labelBot: 'factr.bot',
  labelVersion: 'factr.computer.v',
  version: '1',
  /**
   * The engine serves on $FACTR_ENGINE_PORT (image default 8642) and rejects a Host header whose port differs from
   * the one it is bound to. So the app picks a free loopback port P at create time and publishes `127.0.0.1:P:P`
   * with the engine told to bind P: the Host header `127.0.0.1:P` matches. P is fixed at create (it survives
   * stop/start) and recorded in the `factr.port` label.
   */
  labelPort: 'factr.port',
  /** Hash of the run flags (limits, mounts, env, network): a changed spec recreates a stopped container. */
  labelSpec: 'factr.spec',
  envPort: 'FACTR_ENGINE_PORT',
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
  /** Enabled cron jobs the bot holds: while there are any the computer is exempt from the idle stop. */
  scheduledJobs?: number
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
  /** Test seam: a different container name prefix, so real-Docker tests never share names with the app's computers. */
  namePrefix?: string
  /** Test seam: extra `docker create` args placed before the image (for example a dev bind mount of the backend). */
  extraCreateArgs?: string[]
  /** A free loopback port for a new container (default: listen on 127.0.0.1:0, read it, close). */
  pickPort?: () => Promise<number>
  /** The host IANA time zone, passed as TZ so the bot's clock and cron schedules follow the user's. */
  timeZone?: string
  /** The host engine home (FACTR_HOME, default ~/.factr/engine): where credentials are seeded from. */
  hostEngineHome?: string
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

/**
 * True when `<profile>/profile.yaml` carries `ui_meta: factr-bots: {...}`, the Bot Mode marker the bots plugin writes
 * (the same test as backend/tools/bot_mode_probe.py `_is_bot_managed`). A line scan, since the app has no YAML parser.
 */
export function isBotProfile(configHome: string, profile: string): boolean {
  let lines: string[]

  try {
    lines = fs.readFileSync(path.join(configHome, 'profiles', profile, 'profile.yaml'), 'utf8').split(/\r?\n/)
  } catch {
    return false
  }

  const code = lines.map(l => l.replace(/\s+#.*$/, '')).filter(l => l.trim() && !l.trim().startsWith('#'))
  const start = code.findIndex(l => /^ui_meta\s*:\s*$/.test(l))

  if (start < 0) {
    return false
  }

  let indent = -1

  for (let i = start + 1; i < code.length; i++) {
    const cur = code[i].match(/^(\s*)/)![1].length

    if (cur === 0) {
      return false
    }

    indent = indent < 0 ? cur : indent

    const m = cur === indent && code[i].trim().match(/^["']?factr-bots["']?\s*:\s*(.*)$/)

    if (m) {
      // A dict: inline `{...}`, or a nested block on the next line. `null`, `~` or a scalar is not.
      return m[1].startsWith('{') || (!m[1] && (code[i + 1]?.match(/^(\s*)/)![1].length ?? 0) > cur)
    }
  }

  return false
}

export interface BotRouteFacts {
  /** The packaged Rust engine is primary: one profile per process (ProfileRouteOptions.primarySingleProfile). */
  primarySingleProfile?: boolean
  primaryProfile?: null | string
  globalRemote?: boolean
  profileRemoteOverride?: boolean
  primaryRemoteActive?: boolean
  /** The profile is a Bot Mode bot (isBotProfile). Only bots get a computer. */
  isBot?: boolean
  /** FACTR_BOT_COMPUTERS=0 turns computers off (per-profile host engines instead). */
  disabled?: boolean
}

/**
 * Which backend a LOCAL non-primary profile gets once the router sent it to the pool (case 6 refuses to share a
 * single-profile primary): a Bot Mode bot gets its own computer when Docker is up; every other case (no Docker, not a
 * bot, computers disabled) gets its own host engine. Never both.
 * `null`: not this module's case (primary profile, remote, dev Python gateway), the caller keeps its normal path.
 */
export function chooseBotBackend(
  profile: string,
  facts: BotRouteFacts,
  dockerUp: boolean
): 'container' | 'host-engine' | null {
  const local =
    facts.primarySingleProfile &&
    !facts.globalRemote &&
    !facts.profileRemoteOverride &&
    !facts.primaryRemoteActive &&
    profile !== (facts.primaryProfile || 'default') &&
    PROFILE_RE.test(profile) &&
    profile !== 'default'

  if (!local) {
    return null
  }

  return facts.isBot && !facts.disabled && dockerUp ? 'container' : 'host-engine'
}

/**
 * Every bot that has a computer (its `.computer` marker exists), and whether it is running. Sync and host-only: the
 * relay uses it to treat each computer as its own connection without waking any of them.
 */
export function listComputers(): { profile: string; running: boolean }[] {
  const root = path.join(cfg().configHome, 'profiles')

  let names: string[] = []

  try {
    names = fs.readdirSync(root)
  } catch {
    // No profiles folder yet.
  }

  return names
    .filter(name => PROFILE_RE.test(name) && fs.existsSync(path.join(root, name, BOT_COMPUTER.marker)))
    .map(profile => ({ profile, running: computerActive(profile) }))
}

/**
 * How many enabled cron jobs the bot's own engine holds, read from its job store in the profile folder
 * (`<profile>/cron/jobs.json`, host-readable). The gate is the engine's own due-scan (cron_tick.rs): a job fires unless
 * it is disabled, paused (state or marker), completed, or a one-shot in error. Never throws.
 */
export function enabledCronJobs(profile: string): number {
  try {
    const data = JSON.parse(fs.readFileSync(path.join(hostProfileDir(validateBotProfile(profile)), 'cron', 'jobs.json'), 'utf8'))
    const jobs: unknown[] = Array.isArray(data) ? data : Array.isArray(data?.jobs) ? data.jobs : []

    return jobs.filter((job: any) => {
      if (!job || typeof job !== 'object' || job.enabled === false || job.paused_at || job.state === 'paused') {
        return false
      }

      return job.state !== 'completed' && !(job.state === 'error' && job.schedule?.kind === 'once')
    }).length
  } catch {
    return 0
  }
}

/** A computer that is running (or being started) for this bot. Sync, for routing decisions. */
export const computerActive = (profile: string) => tracked.has(profile) || starting.has(profile)

export const computerName = (profile: string) =>
  `${deps?.namePrefix ?? BOT_COMPUTER.namePrefix}${validateBotProfile(profile)}`

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
  /** The engine port from the `factr.port` label ('' on a container created without one). */
  port: string
  /** The image id the container was created from, and its `factr.spec` label ('' when absent). */
  imageId: string
  spec: string
}

async function inspect(name: string): Promise<Inspect | null> {
  try {
    const out = await docker([
      'inspect',
      '--format',
      `{{.State.Status}}|{{.Config.Image}}|{{index .Config.Labels "${BOT_COMPUTER.labelBot}"}}|{{.State.StartedAt}}|{{index .Config.Labels "${BOT_COMPUTER.labelPort}"}}|{{.Image}}|{{index .Config.Labels "${BOT_COMPUTER.labelSpec}"}}`,
      name
    ])

    const [status = '', image = '', botLabel = '', startedAt = '', port = '', imageId = '', spec = ''] = out
      .trim()
      .split('|')

    return { status, image, botLabel, startedAt, port, imageId, spec }
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

/**
 * One-time credential seed: copy the host engine home's auth files into the bot's own engine dir
 * (`<profile>/engine`), mode 0600. Only when the bot has none of them yet; never overwrites, never mounts the real
 * dir. Returns the files copied.
 * ponytail: if a provider rotates single-use refresh tokens, a bot's refresh can sign the host out (and the reverse).
 * API keys through the profile `.env` avoid that. Upgrade path: per-bot OAuth sign-in inside the computer.
 */
export function seedBotAuth(hostEngineHome: string, profileDir: string): string[] {
  const botEngine = path.join(profileDir, BOT_COMPUTER.engineDir)
  const has = (dir: string, f: string) => fs.existsSync(path.join(dir, f))

  if (BOT_COMPUTER.authFiles.some(f => has(botEngine, f))) {
    return []
  }

  const copied: string[] = []

  for (const f of BOT_COMPUTER.authFiles.filter(f => has(hostEngineHome, f))) {
    try {
      fs.mkdirSync(botEngine, { recursive: true })
      fs.copyFileSync(path.join(hostEngineHome, f), path.join(botEngine, f), fs.constants.COPYFILE_EXCL)
      fs.chmodSync(path.join(botEngine, f), 0o600)
      copied.push(f)
    } catch {
      // Best effort: a bot without seeded credentials still starts (API keys come through its .env).
    }
  }

  return copied
}

let imageCheck: { at: number; ok: boolean } | null = null

/** Is the bot computer image present? Cached briefly (routing asks on every dial); `fresh` skips the cache. */
export async function computerImageReady(fresh = false): Promise<boolean> {
  if (!fresh && imageCheck && now() - imageCheck.at < BOT_COMPUTER.imageCheckMs) {
    return imageCheck.ok
  }

  let ok = true

  try {
    await docker(['image', 'inspect', '--format', '{{.Id}}', computerImage()])
  } catch {
    ok = false
  }

  imageCheck = { at: now(), ok }

  return ok
}

/** The id of the image behind the tag, or null when absent. */
async function currentImageId(): Promise<null | string> {
  try {
    return (await docker(['image', 'inspect', '--format', '{{.Id}}', computerImage()])).trim() || null
  } catch {
    return null
  }
}

/**
 * The routing answer for one profile, with Docker or the image missing treated alike: no computer, so the bot falls
 * back to its own host engine and still chats. Docker and the image are only asked about for a would-be bot. A
 * computer already running keeps the container route even if its image tag vanished (never host engine + container).
 */
export async function botBackendFor(profile: string, facts: BotRouteFacts) {
  if (chooseBotBackend(profile, facts, true) !== 'container') {
    return chooseBotBackend(profile, facts, false)
  }

  return chooseBotBackend(profile, facts, (await dockerAvailable()) && (computerActive(profile) || (await computerImageReady())))
}

/** The exact `docker create` argv. Exported for tests. */
export function createArgs(
  profile: string,
  image: string,
  hostDir: string,
  port: number,
  extra: string[] = [],
  timeZone?: string
): string[] {
  return ['create', ...runFlags(profile, image, hostDir, port, extra, timeZone, computeSpec(profile, image, hostDir, extra, timeZone))]
}

/**
 * Hash of every run flag but the port (which is picked per create). Stored as the `factr.spec` label, so a changed
 * limit, mount, env or network is visible on an existing container without comparing flags.
 */
export function computeSpec(profile: string, image: string, hostDir: string, extra: string[] = [], timeZone?: string) {
  return crypto
    .createHash('sha256')
    .update(runFlags(profile, image, hostDir, 0, extra, timeZone, '').join('\0'))
    .digest('hex')
    .slice(0, 16)
}

function runFlags(
  profile: string,
  image: string,
  hostDir: string,
  port: number,
  extra: string[],
  timeZone: string | undefined,
  spec: string
) {
  const name = computerName(profile)
  const L = BOT_COMPUTER.limits

  return [
    '--name', name,
    '--label', `${BOT_COMPUTER.labelBot}=${profile}`,
    '--label', `${BOT_COMPUTER.labelVersion}=${BOT_COMPUTER.version}`,
    '--label', `${BOT_COMPUTER.labelPort}=${port}`,
    '--label', `${BOT_COMPUTER.labelSpec}=${spec}`,
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
    '-p', `127.0.0.1:${port}:${port}`,
    '-e', `${BOT_COMPUTER.envPort}=${port}`,
    '-v', `${hostDir}:${BOT_COMPUTER.mountRoot}/${profile}`,
    '-e', `${BOT_COMPUTER.envBot}=${profile}`,
    ...(timeZone && /^[A-Za-z0-9_+\-/]{1,64}$/.test(timeZone) ? ['-e', `TZ=${timeZone}`] : []),
    // Name only: the value rides the docker client's env, so it is not in argv (`ps`).
    '-e', BOT_COMPUTER.envToken,
    // A localhost model provider (Ollama :11434, LM Studio :1234) works unchanged: the image's entrypoint forwards
    // those ports to host.docker.internal.
    // ponytail: Linux hosts need --user <host uid> for the bind mount; Docker Desktop (Mac/Windows) maps ownership.
    ...extra,
    image
  ]
}

const markerPath = (profile: string) => path.join(hostProfileDir(profile), BOT_COMPUTER.marker)

function writeMarker(profile: string) {
  fs.writeFileSync(markerPath(profile), JSON.stringify({ container: computerName(profile), v: BOT_COMPUTER.version }))
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as net.AddressInfo
      server.close(() => resolve(port))
    })
  })
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

  const idle = running
    .filter(([p, t]) => t.holds.size === 0 && enabledCronJobs(p) === 0)
    .sort((a, b) => a[1].lastActive - b[1].lastActive)

  // Every running computer is busy or scheduled: exceed the soft cap rather than kill a live session or a schedule.
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
      let info = await inspect(name)

      if (info && info.botLabel !== profile) {
        throw new Error(`A container named ${name} exists but does not belong to this bot; refusing to use it.`)
      }

      // The profile folder (the bot's state) outlives the container, so recreating one loses nothing.
      const create = async () => {
        const hostDir = hostProfileDir(profile)

        // Never recreate a deleted profile folder: a missing folder means the bot is gone.
        if (!fs.existsSync(hostDir)) {
          throw new Error(`Profile folder for "${profile}" does not exist.`)
        }

        // No image, no create: the caller falls back to a host engine (routing) or shows the error (start).
        if (!(await computerImageReady(true))) {
          throw new Error('image-missing')
        }

        const port = await (deps?.pickPort || freePort)()
        await ensureNetwork()

        if (deps?.hostEngineHome) {
          seedBotAuth(deps.hostEngineHome, hostDir)
        }

        await docker(createArgs(profile, image, hostDir, port, cfg().extraCreateArgs, cfg().timeZone), {
          env: { [BOT_COMPUTER.envToken]: token }
        })

        return String(port)
      }

      // A container without a port label predates the fixed-port contract: recreate it.
      if (info && !info.port) {
        await docker(['rm', '-f', name])
        info = null
      }

      // A STOPPED container whose run flags (the `factr.spec` label) or image id no longer match is recreated, so a
      // new limit, mount, time zone or image build takes effect; the profile folder keeps all state. A running one is
      // left alone (recreating would kill its turn and schedules) and picks the change up after its next stop.
      if (info && info.status !== 'running') {
        const wantImage = await currentImageId()

        const wantSpec = computeSpec(profile, image, hostProfileDir(profile), cfg().extraCreateArgs, cfg().timeZone)

        if (info.spec !== wantSpec || (wantImage && info.imageId !== wantImage)) {
          await docker(['rm', '-f', name])
          info = null
        }
      }

      let port = info ? info.port : await create()

      // The Python relay refuses host-side delivery into a bot whose container exists (second SQLite writer).
      writeMarker(profile)

      if (info?.status !== 'running') {
        await stopForCap(profile)

        try {
          await docker(['start', name])
        } catch (error: any) {
          // The published port was taken since create: pick a new one (the mapping is fixed at create).
          if (!/port is already allocated|address already in use|ports are not available|bind for/i.test(`${error?.stderr || ''}${error?.message || ''}`)) {
            throw error
          }

          await docker(['rm', '-f', name])
          port = await create()
          await docker(['start', name])
        }
      }

      const url = `http://127.0.0.1:${port}`
      await cfg().waitReady(url, token)
      track(profile)

      return { url, token, port: Number(port) }
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
  // "Working" means a turn is running; a watched screen ('view') keeps the computer awake but is not work.
  const busy = tracked.get(profile)?.holds.has('turn') ?? false
  const probe = await probeDocker()

  if (!probe.value) {
    return { state: 'no-docker', busy, reason: probe.reason }
  }

  try {
    const info = await inspect(computerName(profile))

    if (!info || info.status !== 'running') {
      if (starting.has(profile)) {
        return { state: 'starting', busy }
      }

      return (await computerImageReady())
        ? { state: 'off', busy }
        : { state: 'error', busy, error: 'image-missing' }
    }

    if (starting.has(profile)) {
      return { state: 'starting', busy }
    }

    const out: ComputerStatus = { state: 'on', busy, startedAt: info.startedAt, scheduledJobs: enabledCronJobs(profile) }

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
    `{{.Names}}|{{.State}}|{{.Label "${BOT_COMPUTER.labelBot}"}}`
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

  // Bots with enabled jobs run first-class: their schedules only fire while their computer is up.
  await startScheduledComputers(keep)

  // The shared network is ours too: drop it once the last bot computer is gone (docker refuses while one is attached).
  if (removed.length > 0 && removed.length === lines.length) {
    await docker(['network', 'rm', BOT_COMPUTER.network]).catch(() => {})
  }

  return removed
}

/**
 * Start the computer of every bot that has one and holds an enabled job (app start). They count against the running
 * cap like any other, but never as its victims (see stopForCap), so a full cap evicts an unscheduled bot instead.
 */
export async function startScheduledComputers(profiles: Iterable<string>): Promise<string[]> {
  const started: string[] = []

  for (const profile of profiles) {
    if (
      PROFILE_RE.test(profile) &&
      profile !== 'default' &&
      fs.existsSync(markerPath(profile)) &&
      !computerActive(profile) &&
      enabledCronJobs(profile) > 0
    ) {
      try {
        await ensureComputer(profile)
        started.push(profile)
      } catch {
        // Docker down or image missing: the bot's schedules wait; status reports why.
      }
    }
  }

  return started
}

/**
 * App quit: stop every computer except those with enabled jobs. Those keep running (the container outlives the app) so
 * their schedules fire; the next app start adopts them (reconcile) and the idle sweep still never stops them.
 */
export async function stopUnscheduledComputers(): Promise<void> {
  await Promise.all(
    [...tracked.keys()].filter(profile => enabledCronJobs(profile) === 0).map(profile => stopComputer(profile).catch(() => {}))
  )
}

async function sweepIdle() {
  const cutoff = now() - BOT_COMPUTER.idleMs

  for (const [profile, t] of [...tracked]) {
    // A bot with an enabled job keeps running: its schedules fire only while its computer is up.
    if (t.holds.size === 0 && t.lastActive < cutoff && enabledCronJobs(profile) === 0) {
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
  imageCheck = null
  tracked.clear()
  starting.clear()
  locks.clear()

  if (sweeper) {
    clearInterval(sweeper)
    sweeper = null
  }
}

export { sweepIdle as sweepIdleComputers }
