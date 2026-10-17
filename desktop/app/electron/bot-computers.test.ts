import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const execFileMock = vi.hoisted(() => vi.fn())

vi.mock('node:child_process', () => ({ execFile: execFileMock }))

import {
  BOT_COMPUTER,
  botBackendFor,
  chooseBotBackend,
  computerImageReady,
  computerStatus,
  configureBotComputers,
  createArgs,
  deriveBotToken,
  destroyComputer,
  dockerAvailable,
  ensureComputer,
  isBotProfile,
  reconcileComputers,
  resetBotComputersForTest,
  seedBotAuth,
  setComputerHold,
  stopComputer,
  sweepIdleComputers,
  validateBotProfile
} from './bot-computers'

const NO_SUCH = Object.assign(new Error('failed'), { stderr: 'Error: No such object: x' })
const IMAGE = 'bots-factr-bot-computer:dev'

let home: string
let calls: { args: string[]; env?: Record<string, string> }[]
let clock: number
let stopped: string[]
let existing: Record<string, { status: string; label: string; port: string }>
let nextPort: number
let startError: Error | null
let imageMissing: boolean
let ps: string

function fakeRun(args: string[], opts?: { env?: Record<string, string> }): Promise<string> {
  calls.push({ args, env: opts?.env })
  const [cmd, sub] = args

  if (cmd === 'version') {
    return Promise.resolve('29.8.0')
  }

  if (cmd === 'image') {
    return imageMissing ? Promise.reject(NO_SUCH) : Promise.resolve('sha256:abc')
  }

  if (cmd === 'inspect') {
    const name = args[args.length - 1]
    const c = existing[name]

    return c
      ? Promise.resolve(`${c.status}|${IMAGE}|${c.label}|2026-10-18T10:00:00Z|${c.port}`)
      : Promise.reject(NO_SUCH)
  }

  if (cmd === 'network' && sub === 'inspect') {
    return Promise.reject(NO_SUCH)
  }

  if (cmd === 'create') {
    const name = args[args.indexOf('--name') + 1]
    const port = args.find(a => a.startsWith('factr.port='))?.slice('factr.port='.length) ?? ''
    existing[name] = { status: 'created', label: name.slice(BOT_COMPUTER.namePrefix.length), port }
  }

  if (cmd === 'start') {
    if (startError) {
      const error = startError
      startError = null

      return Promise.reject(error)
    }

    existing[args[1]].status = 'running'
  }

  if (cmd === 'stop') {
    if (!existing[args[args.length - 1]]) {
      return Promise.reject(NO_SUCH)
    }

    existing[args[args.length - 1]].status = 'exited'
  }

  if (cmd === 'rm') {
    delete existing[args[args.length - 1]]
  }

  if (cmd === 'stats') {
    return Promise.resolve('512MiB / 2GiB\n')
  }

  if (cmd === 'ps') {
    return Promise.resolve(ps)
  }

  return Promise.resolve('')
}

const waitReady = vi.fn(async () => undefined)

function configure(extra: Partial<Parameters<typeof configureBotComputers>[0]> = {}) {
  configureBotComputers({
    configHome: home,
    masterSecret: () => 'master-secret-1',
    waitReady,
    run: fakeRun,
    pickPort: async () => nextPort++,
    now: () => clock,
    onStopped: profile => stopped.push(profile),
    ...extra
  })
}

const dockerCalls = (cmd: string) => calls.filter(c => c.args[0] === cmd)

beforeEach(() => {
  resetBotComputersForTest()
  home = fs.mkdtempSync(path.join(os.tmpdir(), 'bot-computers-'))

  for (const p of ['scout', 'writer', 'a', 'b']) {
    fs.mkdirSync(path.join(home, 'profiles', p), { recursive: true })
  }

  calls = []
  clock = 1_000_000
  stopped = []
  existing = {}
  nextPort = 41000
  startError = null
  imageMissing = false
  ps = ''
  waitReady.mockClear()
})

afterEach(() => {
  resetBotComputersForTest()
  fs.rmSync(home, { recursive: true, force: true })
})

describe('profile name validation', () => {
  it('rejects anything that could reach a docker arg unsafely', () => {
    for (const bad of ['', 'default', '../x', 'a b', 'Scout', 'x;rm -rf', '-v', 'a/b', 'a'.repeat(65), 'x\nrm', null, 7]) {
      expect(() => validateBotProfile(bad)).toThrow(/Invalid bot profile name/)
    }

    expect(validateBotProfile('scout')).toBe('scout')
    expect(validateBotProfile('a_b-1')).toBe('a_b-1')
  })

  it('every entry point validates before any docker call', async () => {
    configure()
    await expect(ensureComputer('../evil')).rejects.toThrow(/Invalid/)
    await expect(stopComputer('a b')).rejects.toThrow(/Invalid/)
    await expect(destroyComputer('default')).rejects.toThrow(/Invalid/)
    await expect(computerStatus('X')).rejects.toThrow(/Invalid/)
    expect(calls).toEqual([])
  })
})

describe('token derivation', () => {
  it('is HMAC per profile: stable, distinct per bot and per secret, 64 hex', () => {
    const t = deriveBotToken('scout', 'secret')

    expect(t).toMatch(/^[0-9a-f]{64}$/)
    expect(deriveBotToken('scout', 'secret')).toBe(t)
    expect(deriveBotToken('writer', 'secret')).not.toBe(t)
    expect(deriveBotToken('scout', 'other')).not.toBe(t)
  })
})

describe('docker create args', () => {
  it('has every hardening flag, a loopback-only publish and no token in argv', () => {
    configure()
    const args = createArgs('scout', IMAGE, '/h/profiles/scout', 41234)

    expect(args).toEqual([
      'create',
      '--name', 'bots-factr-bot-scout',
      '--label', 'factr.bot=scout',
      '--label', 'factr.computer.v=1',
      '--label', 'factr.port=41234',
      '--network', 'bots-factr-net',
      '--add-host', 'host.docker.internal:host-gateway',
      '--cap-drop', 'ALL',
      '--security-opt', 'no-new-privileges',
      '--pids-limit', '2048',
      '--memory', '2g',
      '--cpus', '2',
      '--shm-size', '512m',
      '-p', '127.0.0.1:41234:41234',
      '-e', 'FACTR_ENGINE_PORT=41234',
      '-v', '/h/profiles/scout:/data/profiles/scout',
      '-e', 'FACTR_BOT=scout',
      '-e', 'FACTR_ENGINE_TOKEN',
      IMAGE
    ])
    expect(args.join(' ')).not.toContain(deriveBotToken('scout'))
  })
})

describe('ensureComputer', () => {
  it('creates on a picked free port, starts, waits for health on it, and marks the profile', async () => {
    configure()
    const handle = await ensureComputer('scout')
    const token = deriveBotToken('scout')

    expect(handle).toEqual({ url: 'http://127.0.0.1:41000', token, port: 41000 })
    expect(waitReady).toHaveBeenCalledWith('http://127.0.0.1:41000', token)

    const create = dockerCalls('create')[0]
    expect(create.args).toEqual(createArgs('scout', IMAGE, path.join(home, 'profiles', 'scout'), 41000))
    // The token travels in the docker client's env (by name), never argv.
    expect(create.env).toEqual({ FACTR_ENGINE_TOKEN: token })

    const netCreate = calls.find(c => c.args[0] === 'network' && c.args[1] === 'create')
    expect(netCreate?.args).toEqual([
      'network', 'create', '-o', 'com.docker.network.bridge.enable_icc=false', 'bots-factr-net'
    ])
    expect(dockerCalls('start')[0].args).toEqual(['start', 'bots-factr-bot-scout'])
    expect(dockerCalls('port')).toEqual([])

    const marker = JSON.parse(fs.readFileSync(path.join(home, 'profiles', 'scout', '.computer'), 'utf8'))
    expect(marker.container).toBe('bots-factr-bot-scout')
  })

  it('reuses a running container and its labelled port without creating or starting', async () => {
    configure()
    existing['bots-factr-bot-scout'] = { status: 'running', label: 'scout', port: '43210' }
    expect(await ensureComputer('scout')).toMatchObject({ url: 'http://127.0.0.1:43210', port: 43210 })
    expect(dockerCalls('create')).toEqual([])
    expect(dockerCalls('start')).toEqual([])
  })

  it('keeps the same port across stop and start', async () => {
    configure()
    const first = await ensureComputer('scout')
    await stopComputer('scout')
    const again = await ensureComputer('scout')

    expect(again.port).toBe(first.port)
    expect(dockerCalls('create')).toHaveLength(1)
    expect(dockerCalls('start')).toHaveLength(2)
  })

  it('recreates on a new port when the published port was taken, and never on other start errors', async () => {
    configure()
    await ensureComputer('scout')
    await stopComputer('scout')
    calls = []
    startError = Object.assign(new Error('x'), {
      stderr: 'Error response from daemon: Bind for 127.0.0.1:41000 failed: port is already allocated'
    })

    expect(await ensureComputer('scout')).toMatchObject({ port: 41001 })
    expect(dockerCalls('rm')[0].args).toEqual(['rm', '-f', 'bots-factr-bot-scout'])
    expect(dockerCalls('create')[0].args).toContain('factr.port=41001')
    expect(dockerCalls('start')).toHaveLength(2)

    await stopComputer('scout')
    startError = Object.assign(new Error('x'), { stderr: 'some other failure' })
    await expect(ensureComputer('scout')).rejects.toThrow('x')
  })

  it('recreates a container that has no port label (older create)', async () => {
    configure()
    existing['bots-factr-bot-scout'] = { status: 'exited', label: 'scout', port: '' }
    expect(await ensureComputer('scout')).toMatchObject({ port: 41000 })
    expect(dockerCalls('rm')).toHaveLength(1)
  })

  it('fails clearly without docker, without a profile folder, and on a foreign container', async () => {
    configure({
      run: async args => {
        throw Object.assign(new Error('x'), { code: 'ENOENT', args })
      }
    })
    await expect(ensureComputer('scout')).rejects.toThrow(/Docker is not available/)

    configure()
    fs.rmSync(path.join(home, 'profiles', 'writer'), { recursive: true })
    await expect(ensureComputer('writer')).rejects.toThrow(/does not exist/)
    expect(dockerCalls('create')).toEqual([])

    existing['bots-factr-bot-a'] = { status: 'running', label: 'someone-else', port: '1' }
    await expect(ensureComputer('a')).rejects.toThrow(/does not belong/)
  })

  it('stops the least recently used idle computer when the cap is reached, never a held one', async () => {
    configure({ maxRunning: () => 2 })
    await ensureComputer('scout')
    clock += 1000
    await ensureComputer('writer')
    setComputerHold('scout', 'view', true)
    clock += 1000
    await ensureComputer('a')

    // scout is older but watched: writer goes.
    expect(dockerCalls('stop').map(c => c.args.at(-1))).toEqual(['bots-factr-bot-writer'])
    expect(stopped).toEqual(['writer'])
  })
})

describe('credential seed', () => {
  let host: string
  let profileDir: string

  beforeEach(() => {
    host = fs.mkdtempSync(path.join(os.tmpdir(), 'host-engine-'))
    profileDir = path.join(home, 'profiles', 'scout')
  })

  afterEach(() => fs.rmSync(host, { recursive: true, force: true }))

  it('copies whichever auth files exist into <profile>/engine with mode 0600, and nothing else', () => {
    fs.writeFileSync(path.join(host, 'auth.json'), '{"a":1}', { mode: 0o644 })
    fs.writeFileSync(path.join(host, 'google_oauth.json'), '{"g":1}')
    fs.writeFileSync(path.join(host, 'factr.db'), 'sessions')

    expect(seedBotAuth(host, profileDir).sort()).toEqual(['auth.json', 'google_oauth.json'])
    expect(fs.readdirSync(path.join(profileDir, 'engine')).sort()).toEqual(['auth.json', 'google_oauth.json'])
    expect(fs.readFileSync(path.join(profileDir, 'engine', 'auth.json'), 'utf8')).toBe('{"a":1}')
    expect(fs.statSync(path.join(profileDir, 'engine', 'auth.json')).mode & 0o777).toBe(0o600)
  })

  it('never overwrites: a bot that already has any auth file is left alone', () => {
    fs.mkdirSync(path.join(profileDir, 'engine'), { recursive: true })
    fs.writeFileSync(path.join(profileDir, 'engine', 'openai-auth.json'), 'mine')
    fs.writeFileSync(path.join(host, 'auth.json'), 'host')
    fs.writeFileSync(path.join(host, 'openai-auth.json'), 'host')

    expect(seedBotAuth(host, profileDir)).toEqual([])
    expect(fs.readFileSync(path.join(profileDir, 'engine', 'openai-auth.json'), 'utf8')).toBe('mine')
    expect(fs.existsSync(path.join(profileDir, 'engine', 'auth.json'))).toBe(false)
  })

  it('creates nothing when the host has no credentials', () => {
    expect(seedBotAuth(host, profileDir)).toEqual([])
    expect(fs.existsSync(path.join(profileDir, 'engine'))).toBe(false)
  })

  it('seeds on first create only, and never mounts an engine home', async () => {
    fs.writeFileSync(path.join(host, 'auth.json'), 'host-v1')
    configure({ hostEngineHome: host })
    await ensureComputer('scout')

    expect(fs.readFileSync(path.join(profileDir, 'engine', 'auth.json'), 'utf8')).toBe('host-v1')
    expect(dockerCalls('create')[0].args.join(' ')).not.toMatch(/engine-home|:ro/)

    // The bot refreshed its own copy; a later recreate must not clobber it.
    fs.writeFileSync(path.join(profileDir, 'engine', 'auth.json'), 'bot-refreshed')
    fs.writeFileSync(path.join(host, 'auth.json'), 'host-v2')
    await destroyComputer('scout')
    await ensureComputer('scout')
    expect(fs.readFileSync(path.join(profileDir, 'engine', 'auth.json'), 'utf8')).toBe('bot-refreshed')
  })
})

describe('image missing', () => {
  it('start refuses with image-missing and never tries docker create', async () => {
    configure()
    imageMissing = true
    await expect(ensureComputer('scout')).rejects.toThrow('image-missing')
    expect(dockerCalls('create')).toEqual([])
    expect(dockerCalls('start')).toEqual([])
    expect(fs.existsSync(path.join(home, 'profiles', 'scout', '.computer'))).toBe(false)
  })

  it('status is an image-missing error, but a running computer stays on', async () => {
    configure()
    imageMissing = true
    expect(await computerStatus('scout')).toMatchObject({ state: 'error', error: 'image-missing' })

    existing['bots-factr-bot-scout'] = { status: 'running', label: 'scout', port: '41000' }
    expect((await computerStatus('scout')).state).toBe('on')
  })

  it('caches the image check for 30 seconds, and start always re-checks', async () => {
    configure()
    imageMissing = true
    expect(await computerImageReady()).toBe(false)
    imageMissing = false
    expect(await computerImageReady()).toBe(false) // cached
    expect(dockerCalls('image')).toHaveLength(1)

    clock += 31_000
    expect(await computerImageReady()).toBe(true)

    imageMissing = true
    expect(await computerImageReady()).toBe(true) // cached ok
    await expect(ensureComputer('scout')).rejects.toThrow('image-missing') // fresh
  })

  it('routing falls back to the per-bot host engine, like no Docker', async () => {
    const facts = { primarySingleProfile: true, primaryProfile: 'default', isBot: true }
    configure()
    expect(await botBackendFor('scout', facts)).toBe('container')

    resetBotComputersForTest()
    configure()
    imageMissing = true
    expect(await botBackendFor('scout', facts)).toBe('host-engine')

    resetBotComputersForTest()
    configure({ run: () => Promise.reject(Object.assign(new Error('nope'), { code: 'ENOENT' })) })
    expect(await botBackendFor('scout', facts)).toBe('host-engine')
  })

  it('does not ask docker about a non-bot profile or the primary', async () => {
    configure()
    imageMissing = true
    expect(await botBackendFor('work', { primarySingleProfile: true, primaryProfile: 'default' })).toBe('host-engine')
    expect(await botBackendFor('default', { primarySingleProfile: true, isBot: true })).toBeNull()
    expect(calls).toEqual([])
  })

  it('a computer already running keeps the container route even if its image tag vanished', async () => {
    configure()
    await ensureComputer('scout')
    imageMissing = true
    clock += 31_000
    expect(await botBackendFor('scout', { primarySingleProfile: true, primaryProfile: 'default', isBot: true })).toBe(
      'container'
    )
  })
})

describe('status', () => {
  it('reports no-docker, off, starting, on with memory', async () => {
    configure({ run: () => Promise.reject(Object.assign(new Error('nope'), { code: 'ENOENT' })) })
    expect(await computerStatus('scout')).toMatchObject({ state: 'no-docker', reason: 'not-installed' })

    configure()
    expect(await computerStatus('scout')).toMatchObject({ state: 'off', busy: false })

    let release!: () => void
    waitReady.mockImplementationOnce(() => new Promise<undefined>(r => (release = () => r(undefined))))
    const pending = ensureComputer('scout')
    await vi.waitFor(() => expect(waitReady).toHaveBeenCalled())
    expect((await computerStatus('scout')).state).toBe('starting')
    release()
    await pending

    expect(await computerStatus('scout')).toMatchObject({
      state: 'on',
      memUsedBytes: 512 * 1024 ** 2,
      memLimitBytes: 2 * 1024 ** 3,
      startedAt: '2026-10-18T10:00:00Z'
    })
  })

  it('busy means a turn is running, not a watched screen', async () => {
    configure()
    await ensureComputer('scout')
    setComputerHold('scout', 'view', true)
    expect(await computerStatus('scout')).toMatchObject({ state: 'on', busy: false })
    setComputerHold('scout', 'turn', true)
    expect(await computerStatus('scout')).toMatchObject({ state: 'on', busy: true })
  })
})

describe('stop, destroy, idle', () => {
  it('stop uses a 10s grace and notifies; destroy removes the container and marker but never a network', async () => {
    configure()
    await ensureComputer('scout')
    await stopComputer('scout')
    expect(dockerCalls('stop')[0].args).toEqual(['stop', '-t', '10', 'bots-factr-bot-scout'])
    expect(stopped).toEqual(['scout'])

    await destroyComputer('scout')
    expect(dockerCalls('rm')[0].args).toEqual(['rm', '-f', 'bots-factr-bot-scout'])
    expect(calls.some(c => c.args[0] === 'network' && c.args[1] === 'rm')).toBe(false)
    expect(fs.existsSync(path.join(home, 'profiles', 'scout', '.computer'))).toBe(false)
  })

  it('stopping or destroying a computer that does not exist is a no-op', async () => {
    configure()
    await expect(stopComputer('scout')).resolves.toBeUndefined()
    await expect(destroyComputer('scout')).resolves.toBeUndefined()
  })

  it('idle sweep stops after 30 minutes, but not while held', async () => {
    configure()
    await ensureComputer('scout')
    await ensureComputer('writer')
    setComputerHold('scout', 'turn', true)
    clock += BOT_COMPUTER.idleMs + 1
    await sweepIdleComputers()
    expect(dockerCalls('stop').map(c => c.args.at(-1))).toEqual(['bots-factr-bot-writer'])

    setComputerHold('scout', 'turn', false)
    clock += BOT_COMPUTER.idleMs + 1
    await sweepIdleComputers()
    expect(dockerCalls('stop').map(c => c.args.at(-1))).toEqual(['bots-factr-bot-writer', 'bots-factr-bot-scout'])
  })
})

describe('reconcile', () => {
  it('removes only bots- containers with the factr.bot label whose bot is gone', async () => {
    configure()
    ps = [
      'bots-factr-bot-scout|running|scout',
      'bots-factr-bot-gone|exited|gone',
      // Not ours: wrong prefix (a docker substring-filter false positive), label/name mismatch, no label.
      'other-bots-factr-bot-x|running|x',
      'bots-factr-bot-liar|running|someone',
      'bots-factr-bot-nolabel|running|'
    ].join('\n')
    existing['bots-factr-bot-gone'] = { status: 'exited', label: 'gone', port: '2' }

    expect(await reconcileComputers(['scout'])).toEqual(['gone'])

    const list = dockerCalls('ps')[0].args
    expect(list).toEqual([
      'ps', '-a',
      '--filter', 'label=factr.bot',
      '--filter', 'name=bots-factr-bot-',
      '--format', '{{.Names}}|{{.State}}|{{.Label "factr.bot"}}'
    ])
    expect(dockerCalls('rm').map(c => c.args)).toEqual([['rm', '-f', 'bots-factr-bot-gone']])
    expect(calls.some(c => c.args.includes('prune'))).toBe(false)
  })
})

describe('default docker runner', () => {
  it('calls execFile with an args array (no shell) and passes the token in env, not argv', async () => {
    execFileMock.mockImplementation((_bin, _args, _opts, cb) => cb(null, 'ok', ''))
    process.env.FACTR_DOCKER_BINARY = process.execPath
    resetBotComputersForTest()
    configureBotComputers({ configHome: home, masterSecret: () => 's', waitReady })

    expect(await dockerAvailable()).toBe(true)
    const [bin, args, opts] = execFileMock.mock.calls[0]
    expect(bin).toBe(process.execPath)
    expect(args).toEqual(['version', '--format', '{{.Server.Version}}'])
    expect(opts.shell).toBeUndefined()
    delete process.env.FACTR_DOCKER_BINARY
  })
})

describe('isBotProfile', () => {
  const write = (yaml: string | null) => {
    const dir = path.join(home, 'profiles', 'scout')

    if (yaml === null) {
      return
    }

    fs.writeFileSync(path.join(dir, 'profile.yaml'), yaml)
  }

  it('is true only for a ui_meta -> factr-bots dict in profile.yaml', () => {
    write('description: x\nui_meta:\n  factr-bots:\n    shape: cloud\n    title: Scout\n')
    expect(isBotProfile(home, 'scout')).toBe(true)
    write("ui_meta:\n  'factr-bots': {}\n  other: 1\n")
    expect(isBotProfile(home, 'scout')).toBe(true)
    write('ui_meta:\n  other:\n    a: 1\n  factr-bots:\n    shape: cloud\n')
    expect(isBotProfile(home, 'scout')).toBe(true)
  })

  it('is false for a plain profile, a null marker, a nested lookalike, or no file', () => {
    expect(isBotProfile(home, 'scout')).toBe(false)
    write('description: x\n')
    expect(isBotProfile(home, 'scout')).toBe(false)
    write('ui_meta:\n  factr-bots:\n  other: 1\n')
    expect(isBotProfile(home, 'scout')).toBe(false)
    write('ui_meta:\n  other:\n    factr-bots:\n      a: 1\n')
    expect(isBotProfile(home, 'scout')).toBe(false)
    write('factr-bots:\n  a: 1\nui_meta:\n  x: 1\n')
    expect(isBotProfile(home, 'scout')).toBe(false)
  })
})

describe('routing decision', () => {
  const rust = { primarySingleProfile: true, primaryProfile: 'default' }
  const bot = { ...rust, isBot: true }

  it('Bot Mode bot + Rust engine primary: container when Docker is up, per-profile host engine when not', () => {
    expect(chooseBotBackend('scout', bot, true)).toBe('container')
    expect(chooseBotBackend('scout', bot, false)).toBe('host-engine')
  })

  it('a non-bot local profile gets its own host engine, never a container', () => {
    expect(chooseBotBackend('work', rust, true)).toBe('host-engine')
    expect(chooseBotBackend('work', rust, false)).toBe('host-engine')
  })

  it('leaves everything else alone', () => {
    expect(chooseBotBackend('default', bot, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...bot, primaryProfile: 'scout' }, true)).toBeNull()
    expect(chooseBotBackend('scout', { isBot: true, primaryProfile: 'default' }, true)).toBeNull() // dev Python gateway
    expect(chooseBotBackend('scout', { ...bot, globalRemote: true }, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...bot, profileRemoteOverride: true }, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...bot, primaryRemoteActive: true }, true)).toBeNull()
    expect(chooseBotBackend('../x', bot, true)).toBeNull()
  })

  it('computers off: bots get host engines', () => {
    expect(chooseBotBackend('scout', { ...bot, disabled: true }, true)).toBe('host-engine')
  })
})
