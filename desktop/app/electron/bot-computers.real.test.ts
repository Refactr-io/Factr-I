// REAL Docker + real engines + the local Ollama model (factr/qwen3.8-27b:latest). Skipped unless FACTR_REAL_DOCKER=1.
//
//   FACTR_REAL_DOCKER=1 npx vitest run --project electron electron/bot-computers.real.test.ts
//   FACTR_REAL_OVERLAY=<repo root>  bind-mounts that checkout's backend/ and computer/entrypoint.sh over the image
//                                   (development only; the final run uses the image as built)
//
// Two bot computers on one shared network, driven only through their engines' own APIs at 127.0.0.1:<port> with the
// per-bot token, exactly as the Desktop does. The test plays the Desktop relay (relay.ts) for the bot-to-bot part.
// Containers are named bots-b2b-<profile> (never the app's bots-factr-bot-<profile>) and removed by exact name.
import { execFileSync } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import path from 'node:path'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import {
  computerStatus,
  configureBotComputers,
  deriveBotToken,
  destroyComputer,
  enabledCronJobs,
  ensureComputer,
  stopComputer,
  sweepIdleComputers,
  touchComputer
} from './bot-computers'

const REAL = process.env.FACTR_REAL_DOCKER === '1'
const SCRATCH = process.env.FACTR_REAL_SCRATCH || process.env.SCRATCH || '/private/tmp'
const PREFIX = 'bots-b2b-'
const SECRET = 'real-test-master-secret'
const MIN = 60_000

const suffix = crypto.randomBytes(3).toString('hex')
const A = `ra${suffix}`
const B = `rb${suffix}`
const run = path.join(SCRATCH, `real-${suffix}`)
const nick = `zebra-${crypto.randomBytes(3).toString('hex')}`
const nickA = `otter-${crypto.randomBytes(3).toString('hex')}`
const privateCanary = `private-${crypto.randomBytes(12).toString('hex')}`
let clock = Date.now()
const ports: Record<string, number> = {}

const sh = (args: string[], opts: { input?: string } = {}) =>
  execFileSync('docker', args, { encoding: 'utf8', timeout: 120_000, ...opts })

const exec = (profile: string, cmd: string, env: string[] = []) =>
  sh(['exec', ...env.flatMap(e => ['-e', e]), PREFIX + profile, 'sh', '-c', cmd])

const sleep = (ms: number) => new Promise(r => setTimeout(r, ms))

async function until<T>(what: string, ms: number, fn: () => T | Promise<T>): Promise<NonNullable<T>> {
  const end = Date.now() + ms

  for (;;) {
    try {
      const v = await fn()

      if (v) {
        return v as NonNullable<T>
      }
    } catch {
      // keep polling
    }

    if (Date.now() > end) {
      throw new Error(`timed out waiting for ${what}`)
    }

    await sleep(4000)
  }
}

class Rpc {
  private ws!: WebSocket
  private n = 0
  private pending = new Map<number, { res: (v: any) => void; rej: (e: Error) => void }>()

  constructor(private port: number, private token: string) {}

  async open() {
    this.ws = new WebSocket(`ws://127.0.0.1:${this.port}/api/ws?token=${this.token}`)

    this.ws.onmessage = e => {
      const m = JSON.parse(String(e.data))
      const p = this.pending.get(m.id)

      if (p) {
        this.pending.delete(m.id)
        m.error ? p.rej(Object.assign(new Error(m.error.message), { data: m.error.data })) : p.res(m.result)
      }
    }

    await new Promise((res, rej) => {
      this.ws.onopen = res
      this.ws.onerror = () => rej(new Error('ws error'))
    })

    return this
  }

  call(method: string, params: object = {}, timeoutMs = 120_000): Promise<any> {
    const id = ++this.n

    return new Promise((res, rej) => {
      const t = setTimeout(() => rej(new Error(`${method} timeout`)), timeoutMs)

      this.pending.set(id, { res: v => (clearTimeout(t), res(v)), rej: e => (clearTimeout(t), rej(e)) })
      this.ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
  }

  close() {
    this.ws?.close()
  }
}

const rpcs: Record<string, Rpc> = {}
let senderSession: string | undefined
const rpc = async (profile: string) => (rpcs[profile] ??= await new Rpc(ports[profile], deriveBotToken(profile, SECRET)).open())

function post(profile: string, route: string, body: object, timeoutMs: number): Promise<any> {
  return new Promise((resolve, reject) => {
    const data = JSON.stringify(body)

    const req = http.request(
      {
        host: '127.0.0.1',
        port: ports[profile],
        path: route,
        method: 'POST',
        timeout: timeoutMs,
        headers: {
          'X-Factr-Session-Token': deriveBotToken(profile, SECRET),
          'content-type': 'application/json',
          'content-length': Buffer.byteLength(data)
        }
      },
      res => {
        let text = ''

        res.setEncoding('utf8')
        res.on('data', c => (text += c))
        res.on('end', () => resolve(JSON.parse(text)))
      }
    )

    req.on('timeout', () => req.destroy(new Error('timeout')))
    req.on('error', reject)
    req.write(data)
    req.end()
  })
}

// ── the Desktop relay (relay.ts), played against the engines ───────────────────────────────────────────────
const connectionId = (p: string) => `computer-${p}`
const relayLog: { sender: string; target: string; hops?: number; reply?: string; error?: string }[] = []
let relayTimer: ReturnType<typeof setInterval> | undefined

async function syncRosters() {
  const rows = [A, B].map(p => ({ profile: p, handle: p, connection_id: connectionId(p), connection_label: `${p} computer` }))

  for (const p of [A, B]) {
    const r = await rpc(p)

    // roster.sync is answered by an engine boot stub until its Python backend runs; any real drain starts it.
    await r.call('bot_relay.outbox.drain')
    await r.call('bot_relay.roster.sync', { agents: rows.filter(row => row.profile !== p) })
  }
}

async function deliver(sender: string, env: any) {
  const target = String(env.target_connection).replace(/^computer-/, '')
  const entry: (typeof relayLog)[number] = { sender, target: env.target_profile, hops: env.hops }

  relayLog.push(entry)

  const reply = (x: object) => rpc(sender).then(r => r.call('bot_relay.reply', { id: env.id, ...x }))

  try {
    const t = await rpc(target)
    let res: any

    for (const t0 = Date.now(); Date.now() - t0 < 25 * MIN; ) {
      // The native endpoint joins the same durable delivery id on every poll.
      res = await t.call(
        'bot.chat.deliver',
        {
          profile: env.target_profile,
          text: env.message,
          from_profile: env.from_profile,
          from_handle: env.from_handle,
          from_connection: connectionId(sender),
          hops: env.hops,
          envelope_id: env.id
        },
        130_000
      )

      if (res.status !== 'pending') {
        break
      }
    }

    if (res.status !== 'done' || res.ok !== true) throw new Error(JSON.stringify(res))
    entry.reply = String(res.reply)
    await reply({ reply: String(res.reply || '') })
  } catch (e: any) {
    entry.error = String(e.message)
    await reply({ error: String(e.message), ...(e.data?.reason ? { reason: e.data.reason } : {}) })
  }
}

const startRelay = () => {
  relayTimer = setInterval(async () => {
    for (const p of [A, B]) {
      try {
        const { envelopes = [] } = await (await rpc(p)).call('bot_relay.outbox.drain')

        envelopes.forEach((env: any) => void deliver(p, env))
      } catch {
        // next tick
      }
    }
  }, 2000)
}

function plantProfile(profile: string) {
  const dir = path.join(run, 'profiles', profile)

  fs.mkdirSync(path.join(dir, 'workspace'), { recursive: true })
  fs.writeFileSync(
    path.join(dir, 'config.yaml'),
    `model:
  default: factr/qwen3.8-27b:latest
  provider: ollama
  base_url: http://localhost:11434/v1
approvals:
  mode: 'off'
auxiliary:
  title_generation:
    enabled: false
plugins:
  enabled: []
mcp_servers:
  factr_teammates:
    command: \${FACTR_BACKEND_PYTHON}
    args: ['-m', 'tools.message_agent_mcp']
    env:
      PYTHONPATH: \${FACTR_BACKEND_PYTHONPATH}
    timeout_secs: 10
`
  )
  fs.writeFileSync(path.join(dir, 'profile.yaml'), `description: ${profile} test bot\nui_meta:\n  factr-bots:\n    shape: blobatar\n    title: ${profile}\n`)
  fs.writeFileSync(path.join(dir, 'SOUL.md'), `You are ${profile}, a bot with your own computer. Be brief and literal.\n`)
}

describe.skipIf(!REAL)('two bot computers: bot-to-bot and scheduled work (real Docker, real engines, local model)', () => {
  beforeAll(async () => {
    const overlay = process.env.FACTR_REAL_OVERLAY

    configureBotComputers({
      configHome: run,
      masterSecret: () => SECRET,
      namePrefix: PREFIX,
      maxRunning: () => 5,
      now: () => clock,
      timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      extraCreateArgs: overlay
        ? ['-v', `${overlay}/backend:/opt/factr/backend-python/source:ro`, '-v', `${overlay}/computer/entrypoint.sh:/usr/local/bin/factr-bot-entrypoint:ro`]
        : [],
      waitReady: async (url: string) => {
        await until('engine health', 120_000, async () => (await fetch(`${url}/api/health`)).ok)
      }
    })

    for (const p of [A, B]) {
      plantProfile(p)
      ports[p] = (await ensureComputer(p)).port
    }

    exec(A, `mkdir -p /home/bot/workspace && echo ${nickA} > /home/bot/workspace/nickname.txt`)
    exec(B, `mkdir -p /home/bot/workspace && echo ${nick} > /home/bot/workspace/nickname.txt && echo ${privateCanary} > /home/bot/workspace/private.txt`)
    await syncRosters()
    startRelay()
  }, 15 * MIN)

  afterAll(async () => {
    const diagnostics = path.join(SCRATCH, `bots-real-diagnostics-${suffix}`)
    fs.mkdirSync(diagnostics, { recursive: true })
    fs.writeFileSync(path.join(diagnostics, 'relay.json'), JSON.stringify(relayLog, null, 2))
    for (const p of [A, B]) {
      try {
        const sessions = await (await rpc(p)).call('session.list', {}, 10_000)
        fs.writeFileSync(path.join(diagnostics, `${p}-sessions.json`), JSON.stringify(sessions, null, 2))
        const ids = new Set<string>((sessions.sessions || []).map((session: { id: string }) => session.id))
        if (p === A && senderSession) ids.add(senderSession)
        for (const id of ids) {
          const history = await (await rpc(p)).call('session.history', { session_id: id }, 10_000)
          fs.writeFileSync(path.join(diagnostics, `${p}-${id}.json`), JSON.stringify(history, null, 2))
        }
        const logs = sh(['logs', '--tail', '250', PREFIX + p])
        fs.writeFileSync(path.join(diagnostics, `${p}-engine.log`), logs.replaceAll(deriveBotToken(p, SECRET), '[redacted]'))
      } catch (error) {
        fs.writeFileSync(path.join(diagnostics, `${p}-diagnostic-error.txt`), String(error))
      }
    }
    console.log(`Synthetic diagnostics: ${diagnostics}`)
    clearInterval(relayTimer)
    Object.values(rpcs).forEach(r => r.close())

    for (const p of [A, B]) {
      await destroyComputer(p).catch(() => {})
    }

    // Only what this run created, by exact name.
    for (const p of [A, B]) {
      try {
        sh(['rm', '-f', PREFIX + p])
      } catch {
        // already gone
      }
    }

    // The shared network is removed only if nothing is attached (docker refuses otherwise).
    try {
      sh(['network', 'rm', 'bots-factr-net'])
    } catch {
      // still in use by someone else's computers, or already gone
    }

    fs.rmSync(run, { recursive: true, force: true })
  }, 5 * MIN)

  it('A asks B something only B can answer from its own computer, and gets the answer back', async () => {
    const res = await post(
      A,
      '/api/agent/run',
      {
        prompt: `Use your message_agent tool now. Target: ${B}. Message: "Please read /home/bot/workspace/nickname.txt on YOUR computer using your read tool and tell me the exact text it contains." Finish once the tool queues the message; the reply will wake this chat later.`,
        surface: 'bot',
        session_key: `real-${suffix}`,
        title: 'Bot Chat',
        timeout_s: 1500
      },
      26 * MIN
    )

    // Native message_agent returns queued first; the later reply wakes the original sender session.
    senderSession = res.session_id
    fs.writeFileSync(path.join(SCRATCH, `bots-native-start-${suffix}.json`), JSON.stringify(res, null, 2))
    expect(res.ok).toBe(true)
    const history = await (await rpc(A)).call('session.history', { session_id: res.session_id })
    fs.writeFileSync(path.join(SCRATCH, `bots-native-history-${suffix}.json`), JSON.stringify(history, null, 2))
    const hop = await until('A produces an outbound envelope for B', 30_000, () => relayLog.find(l => l.sender === A && l.target === B))
    await until('B completes the delivered turn', 15 * MIN, () => hop.reply !== undefined || hop.error)
    expect(hop.error).toBeUndefined()
    expect(hop.reply).toContain(nick)
    await until('native reply wakes A', 15 * MIN, async () => {
      const history = await (await rpc(A)).call('session.history', { session_id: res.session_id })
      return JSON.stringify(history).includes(nick)
    })
    // ... and the message really went A -> relay -> B, one hop, with B's own engine producing the reply.
    expect(hop?.hops).toBe(1)
    expect(relayLog.some(l => l.reply?.includes(nick))).toBe(true)
  }, 30 * MIN)

  it('keeps the two computers isolated: own files only, no shared folder, no container-to-container path', () => {
    expect(exec(A, 'cat /home/bot/workspace/nickname.txt').trim()).toBe(nickA)
    expect(exec(B, 'cat /home/bot/workspace/nickname.txt').trim()).toBe(nick)
    // A sees only its own profile folder and none of B's files.
    expect(exec(A, 'ls /data/profiles').trim()).toBe(A)
    expect(exec(A, `grep -rl ${privateCanary} /home/bot /data 2>/dev/null | head -1 || true`).trim()).toBe('')

    // No network path to B: not by container address (inter-container traffic is off on the shared network) ...
    const ipB = sh(['inspect', '-f', '{{(index .NetworkSettings.Networks "bots-factr-net").IPAddress}}', PREFIX + B]).trim()

    expect(() => exec(A, `timeout 4 bash -c '</dev/tcp/${ipB}/${ports[B]}'`)).toThrow()
    // ... and B's token never opens it: B's engine refuses A's token.
    const bad = deriveBotToken(A, SECRET)

    expect(
      execFileSync('curl', ['-s', '-o', '/dev/null', '-w', '%{http_code}', '-H', `X-Factr-Session-Token: ${bad}`, `http://127.0.0.1:${ports[B]}/api/sessions`], { encoding: 'utf8' })
    ).toBe('401')
  }, 60_000)

  it('stops a message chain at the hop cap: a turn already two hops deep cannot relay on', () => {
    const out = exec(
      A,
      `cd /tmp && PYTHONPATH=/opt/factr/backend-python/source:/opt/factr/backend-python/packages FACTR_BOT_COMPUTER=1 FACTR_CONFIG_HOME=/data/profiles/${A} /opt/factr/backend-python/runtime/bin/python3.12 -c "
from pathlib import Path
from tools import message_agent_mcp as m
print(m.deliver('${B}', 'ping', Path('/data/profiles/${A}')))"`,
      ['FACTR_RELAY_HOPS=2']
    )

    expect(out).toContain('hop_limit')
  })

  describe('scheduled work on the bot’s own computer', () => {
    let jobId = ''
    const ticks = () => Number(exec(A, `wc -l < /home/bot/cron-ticks.txt 2>/dev/null || echo 0`).trim())
    const manage = async (action: string, extra: object = {}) => (await rpc(A)).call('cron.manage', { action, ...extra }, 60_000)

    // One computer at a time while the schedule is under test.
    beforeAll(async () => {
      clearInterval(relayTimer)
    })

    it('a job created through the bot’s engine fires every minute and writes its output in its own container', async () => {
      const created = await manage('add', {
        name: 'tick',
        schedule: 'every 1m',
        deliver: 'local',
        prompt: 'Run this shell command and nothing else: echo tick-$(date +%s) >> /home/bot/cron-ticks.txt . Then reply with exactly TICK.'
      })

      jobId = created.job_id
      expect(created.success).toBe(true)
      expect(created.job.enabled).toBe(true)

      await until('first fire', 6 * MIN, () => ticks() >= 1)
      // The run's output is a file in the bot's own profile folder, readable from the host.
      const outDir = path.join(run, 'profiles', A, 'cron', 'output', jobId)

      await until('run output file', 6 * MIN, () => fs.readdirSync(outDir).some(f => f.endsWith('.md')))
      expect(exec(B, 'test -e /home/bot/cron-ticks.txt && echo yes || echo no').trim()).toBe('no') // B never ran it
    }, 20 * MIN)

    it('the host sees the enabled job, so the idle stop leaves the computer running', async () => {
      expect(enabledCronJobs(A)).toBe(1)
      expect((await computerStatus(A)).scheduledJobs).toBe(1)

      clock += 31 * MIN // past the 30 minute idle window
      await sweepIdleComputers()
      expect((await computerStatus(A)).state).toBe('on')
    })

    it('pause stops it firing; resume starts it again', async () => {
      await manage('pause', { name: jobId })
      expect(enabledCronJobs(A)).toBe(0)
      touchComputer(A) // The user just interacted; a paused schedule no longer protects against idle stop.
      // Let any in-flight fire finish, then count over a window longer than two schedule periods.
      await sleep(60_000)

      const paused = ticks()

      await sleep(150_000)
      expect(ticks()).toBe(paused)

      await manage('resume', { name: jobId })
      expect(enabledCronJobs(A)).toBe(1)
      await until('a fire after resume', 6 * MIN, () => ticks() > paused)
    }, 20 * MIN)

    it('jobs persist across docker stop and start, and keep firing', async () => {
      const before = ticks()

      await stopComputer(A)
      expect(sh(['inspect', '-f', '{{.State.Status}}', PREFIX + A]).trim()).toBe('exited')
      expect(enabledCronJobs(A)).toBe(1) // the store is in the profile folder

      ports[A] = (await ensureComputer(A)).port
      rpcs[A]?.close()
      delete rpcs[A]

      const listed = await manage('list')

      expect(listed.jobs.map((j: any) => j.job_id)).toContain(jobId)
      await until('a fire after restart', 8 * MIN, () => ticks() > before)
    }, 25 * MIN)

    it('with the job gone from the store the next idle sweep stops the computer', async () => {
      await manage('remove', { name: jobId })
      expect(enabledCronJobs(A)).toBe(0)
      clock += 31 * MIN
      await sweepIdleComputers()
      expect((await computerStatus(A)).state).toBe('off')
    }, 5 * MIN)
  })
})
