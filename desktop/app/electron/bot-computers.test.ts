import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const execFileMock = vi.hoisted(() => vi.fn())

vi.mock('node:child_process', () => ({ execFile: execFileMock }))

import {
  BOT_COMPUTER,
  chooseBotBackend,
  computerStatus,
  configureBotComputers,
  createArgs,
  deriveBotToken,
  destroyComputer,
  dockerAvailable,
  ensureComputer,
  reconcileComputers,
  resetBotComputersForTest,
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
let existing: Record<string, { status: string; label: string }>
let ps: string

function fakeRun(args: string[], opts?: { env?: Record<string, string> }): Promise<string> {
  calls.push({ args, env: opts?.env })
  const [cmd, sub] = args

  if (cmd === 'version') {
    return Promise.resolve('29.8.0')
  }

  if (cmd === 'inspect') {
    const name = args[args.length - 1]
    const c = existing[name]

    return c ? Promise.resolve(`${c.status}|${IMAGE}|${c.label}|2026-10-18T10:00:00Z`) : Promise.reject(NO_SUCH)
  }

  if (cmd === 'network' && sub === 'inspect') {
    return Promise.reject(NO_SUCH)
  }

  if (cmd === 'create') {
    const name = args[args.indexOf('--name') + 1]
    existing[name] = { status: 'created', label: name.slice(BOT_COMPUTER.namePrefix.length) }
  }

  if (cmd === 'start') {
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

  if (cmd === 'port') {
    return Promise.resolve('0.0.0.0:49153\n127.0.0.1:49153\n')
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
    const args = createArgs('scout', IMAGE, '/h/profiles/scout')

    expect(args).toEqual([
      'create',
      '--name', 'bots-factr-bot-scout',
      '--label', 'factr.bot=scout',
      '--label', 'factr.computer.v=1',
      '--network', 'bots-factr-net',
      '--add-host', 'host.docker.internal:host-gateway',
      '--cap-drop', 'ALL',
      '--security-opt', 'no-new-privileges',
      '--pids-limit', '2048',
      '--memory', '2g',
      '--cpus', '2',
      '--shm-size', '512m',
      '-p', '127.0.0.1:0:8642',
      '-v', '/h/profiles/scout:/data/profiles/scout',
      '-e', 'FACTR_BOT=scout',
      '-e', 'FACTR_ENGINE_TOKEN',
      IMAGE
    ])
    expect(args.join(' ')).not.toContain(deriveBotToken('scout'))
  })

  it('mounts the auth store read-only only when configured', () => {
    const args = createArgs('scout', IMAGE, '/h/p', '/auth')

    expect(args).toContain('/auth:/data/engine-home:ro')
    expect(args).toContain('FACTR_HOME=/data/engine-home')
  })
})

describe('ensureComputer', () => {
  it('creates, starts, reads the published port, waits for health, and marks the profile', async () => {
    configure()
    const handle = await ensureComputer('scout')
    const token = deriveBotToken('scout')

    expect(handle).toEqual({ url: 'http://127.0.0.1:49153', token, port: 49153 })
    expect(waitReady).toHaveBeenCalledWith('http://127.0.0.1:49153', token)

    const create = dockerCalls('create')[0]
    expect(create.args).toEqual(createArgs('scout', IMAGE, path.join(home, 'profiles', 'scout')))
    // The token travels in the docker client's env (by name), never argv.
    expect(create.env).toEqual({ FACTR_ENGINE_TOKEN: token })

    const netCreate = calls.find(c => c.args[0] === 'network' && c.args[1] === 'create')
    expect(netCreate?.args).toEqual([
      'network', 'create', '-o', 'com.docker.network.bridge.enable_icc=false', 'bots-factr-net'
    ])
    expect(dockerCalls('start')[0].args).toEqual(['start', 'bots-factr-bot-scout'])
    expect(dockerCalls('port')[0].args).toEqual(['port', 'bots-factr-bot-scout', '8642/tcp'])

    const marker = JSON.parse(fs.readFileSync(path.join(home, 'profiles', 'scout', '.computer'), 'utf8'))
    expect(marker.container).toBe('bots-factr-bot-scout')
  })

  it('reuses a running container without creating or starting', async () => {
    configure()
    existing['bots-factr-bot-scout'] = { status: 'running', label: 'scout' }
    await ensureComputer('scout')
    expect(dockerCalls('create')).toEqual([])
    expect(dockerCalls('start')).toEqual([])
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

    existing['bots-factr-bot-a'] = { status: 'running', label: 'someone-else' }
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
    existing['bots-factr-bot-gone'] = { status: 'exited', label: 'gone' }

    expect(await reconcileComputers(['scout'])).toEqual(['gone'])

    const list = dockerCalls('ps')[0].args
    expect(list).toEqual([
      'ps', '-a',
      '--filter', 'label=factr.bot',
      '--filter', 'name=bots-factr-bot-',
      '--format', '{{.Names}}|{{.State}}|{{index .Labels "factr.bot"}}'
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

describe('routing decision', () => {
  const rust = { primarySingleProfile: true, primaryProfile: 'default' }

  it('bot profile + Rust engine primary: container when Docker is up, per-profile host engine when not', () => {
    expect(chooseBotBackend('scout', rust, true)).toBe('container')
    expect(chooseBotBackend('scout', rust, false)).toBe('host-engine')
  })

  it('leaves everything else alone', () => {
    expect(chooseBotBackend('default', rust, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...rust, primaryProfile: 'scout' }, true)).toBeNull()
    expect(chooseBotBackend('scout', { primaryProfile: 'default' }, true)).toBeNull() // dev Python gateway multiplexes
    expect(chooseBotBackend('scout', { ...rust, globalRemote: true }, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...rust, profileRemoteOverride: true }, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...rust, primaryRemoteActive: true }, true)).toBeNull()
    expect(chooseBotBackend('scout', { ...rust, disabled: true }, true)).toBeNull()
    expect(chooseBotBackend('../x', rust, true)).toBeNull()
  })
})
