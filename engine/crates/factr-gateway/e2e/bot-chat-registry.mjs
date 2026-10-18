#!/usr/bin/env node
// Real-engine check of the per-bot Bot Chat registry (`bot.chat.get` / `bot.chat.ensure`): two bots, each on its
// OWN engine process (`factr --profile <bot> serve`) with its own FACTR_HOME. Proves: `get` is read-only,
// `ensure` is idempotent, each engine has exactly one Bot Chat, the ids differ across bots, the chat survives an
// engine restart, and it is a real, listed, titled session. No model turn, no network, no credentials.
//
//   cargo build --release --bin factr && node crates/factr-gateway/e2e/bot-chat-registry.mjs
//   FACTR_BIN=<path to factr>  SCRATCH=<dir>  (defaults: target/release/factr, a temp dir)
import { spawn } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const BIN = process.env.FACTR_BIN || path.resolve('target/release/factr')
const scratch = fs.mkdtempSync(path.join(process.env.SCRATCH || os.tmpdir(), 'bot-chat-registry-'))
const cfg = path.join(scratch, 'config')
let failures = 0
const check = (cond, msg) => {
  console.log(cond ? 'ok  ' : 'FAIL', msg)
  if (!cond) failures++
}

const MODEL = 'registry-check:latest'

function seed(bot, sharedHome) {
  fs.mkdirSync(path.join(cfg, 'profiles', bot), { recursive: true })
  fs.mkdirSync(path.join(scratch, bot), { recursive: true })
  const factrHome = sharedHome || path.join(scratch, bot, 'engine')
  fs.mkdirSync(factrHome, { recursive: true })
  // A keyless local endpoint is all a new chat needs to pin its model; no request is ever sent to it.
  fs.writeFileSync(
    path.join(factrHome, 'config.toml'),
    `[provider]\ndefault_provider = "local"\n\n[providers.local]\ntype = "openai-compatible"\nbase_url = "http://127.0.0.1:9/v1"\napi_key = "none"\nrequires_api_key = false\ndefault_model = "${MODEL}"\n\n[[providers.local.models]]\nid = "${MODEL}"\ncontext_window = 8192\n`
  )
  return factrHome
}

async function start(bot, factrHome) {
  const token = crypto.randomBytes(24).toString('hex')
  const runtime = fs.mkdtempSync('/tmp/bcr-') // short private dir: sockets and the daemon lock never collide
  const env = {
    ...process.env,
    HOME: path.join(scratch, bot),
    FACTR_HOME: factrHome,
    FACTR_CONFIG_HOME: cfg,
    FACTR_RUNTIME_DIR: runtime,
    FACTR_DASHBOARD_SESSION_TOKEN: token,
    OLLAMA_HOST: '127.0.0.1:1'
  }
  const child = spawn(BIN, ['--profile', bot, '--provider', 'openai-compatible', '--model', MODEL, 'serve', '--host', '127.0.0.1', '--port', '0'], {
    env,
    cwd: path.join(scratch, bot),
    stdio: ['ignore', 'pipe', 'pipe']
  })
  let stderr = ''
  child.stderr.on('data', d => { stderr += d })
  const port = await new Promise((resolve, reject) => {
    let buf = ''
    child.stdout.on('data', d => {
      buf += d
      const m = buf.match(/FACTR_BACKEND_READY port=(\d+)/)
      if (m) resolve(Number(m[1]))
    })
    child.on('exit', code => reject(new Error(`engine ${bot} exited ${code}\n${stderr}`)))
    setTimeout(() => reject(new Error(`engine ${bot} did not start\n${stderr}`)), 120_000)
  })
  const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`)
  const pending = new Map()
  let next = 1
  ws.addEventListener('message', m => {
    const f = JSON.parse(String(m.data))
    if (f.id !== undefined && pending.has(f.id)) { pending.get(f.id)(f); pending.delete(f.id) }
  })
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve); ws.addEventListener('error', reject) })
  const rpc = (method, params = {}) => {
    const id = `e${next++}`
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    return new Promise(resolve => pending.set(id, resolve))
  }
  const stop = async () => {
    ws.close()
    child.kill('SIGTERM')
    await new Promise(resolve => { child.on('exit', resolve); setTimeout(resolve, 5000) })
  }
  return { rpc, stop, port }
}

const result = async (eng, method, params) => {
  const reply = await eng.rpc(method, params)
  if (reply.error) throw new Error(`${method}: ${JSON.stringify(reply.error)}`)
  return reply.result
}

try {
  const homeA = seed('a')
  const homeB = seed('b')
  let a = await start('a', homeA)
  const b = await start('b', homeB)

  const before = await result(a, 'bot.chat.get')
  check(before.canonical_session === null && before.created === false, 'get on a fresh engine: no Bot Chat, nothing created')
  check((await result(a, 'bot.chat.get')).canonical_session === null, 'get is read-only: still none')

  const first = await result(a, 'bot.chat.ensure')
  const idA = first.canonical_session?.id
  check(first.created === true && typeof idA === 'string' && idA.length > 0, `bot A ensure creates it (${idA})`)
  check(first.canonical_session.title === 'Bot Chat' && first.canonical_session.root_title === 'Bot Chat' && first.canonical_session.resolved_id === idA, 'row has the roster shape (id, resolved_id, title, root_title)')

  const second = await result(a, 'bot.chat.ensure')
  check(second.created === false && second.canonical_session.id === idA, 'bot A ensure again: same id, nothing created')
  check((await result(a, 'bot.chat.get')).canonical_session.id === idA, 'bot A get now finds it')

  // The same call, concurrently, from two windows: still one chat.
  const [c1, c2] = await Promise.all([result(b, 'bot.chat.ensure'), result(b, 'bot.chat.ensure')])
  const idB = c1.canonical_session.id
  check(c1.canonical_session.id === c2.canonical_session.id && [c1.created, c2.created].filter(Boolean).length === 1, `bot B: two concurrent ensures, one chat (${idB})`)
  check(idA !== idB, 'different bots have different Bot Chats')

  const listed = (await result(a, 'session.list', { limit: 100 })).sessions.filter(s => s.title === 'Bot Chat')
  check(listed.length === 1 && listed[0].id === idA, 'the engine lists exactly one "Bot Chat" session for bot A')
  const listedB = (await result(b, 'session.list', { limit: 100 })).sessions.filter(s => s.title === 'Bot Chat')
  check(listedB.length === 1 && listedB[0].id === idB, 'and exactly one for bot B (a never sees b)')
  check(!(await result(a, 'session.list', { limit: 100 })).sessions.some(s => s.id === idB), "bot A's engine does not know bot B's chat")

  await a.stop()
  a = await start('a', homeA)
  const afterRestart = await result(a, 'bot.chat.get')
  check(afterRestart.canonical_session?.id === idA, 'bot A restarted: get returns the same Bot Chat')
  check((await result(a, 'bot.chat.ensure')).created === false, 'bot A restarted: ensure creates nothing')

  await a.stop()
  await b.stop()

  // The packaged layout: every host engine shares ONE FACTR_HOME (one session store). A title alone cannot say
  // whose "Bot Chat" it is there, so the registry is a pointer per profile.
  const shared = path.join(scratch, 'shared-engine-home')
  const homeX = seed('x', shared)
  seed('y', shared)
  seed('z', shared)
  const x = await start('x', homeX)
  const y = await start('y', shared)
  const idX = (await result(x, 'bot.chat.ensure')).canonical_session.id
  const idY = (await result(y, 'bot.chat.ensure')).canonical_session.id
  check(idX !== idY, 'shared engine home: x and y still get different Bot Chats')
  check((await result(x, 'bot.chat.get')).canonical_session.id === idX && (await result(y, 'bot.chat.get')).canonical_session.id === idY, 'shared engine home: each profile gets back its own')
  const both = (await result(x, 'session.list', { limit: 100 })).sessions.filter(r => r.title === 'Bot Chat').map(r => r.id).sort()
  check(both.length === 2 && both.includes(idX) && both.includes(idY), 'shared engine home: the store really holds both chats (a title scan could not tell them apart)')

  // The roster asks ONE engine (the primary) about every host bot, so no bot engine has to start: with the same shared
  // home, a fourth engine ("p", standing in for the primary) answers for x and y without being either of them.
  seed('p', shared)
  const p = await start('p', shared)
  const one = await result(p, 'bot.chat.get', { profile: 'x' })
  check(one.canonical_session?.id === idX, "primary: get {profile:'x'} returns x's chat")
  const many = await result(p, 'bot.chat.get_many', { profiles: ['x', 'y', 'nobody'] })
  check(many.chats.x?.id === idX && many.chats.y?.id === idY && many.chats.nobody === null, 'primary: get_many answers x, y and null for an unregistered profile in one call')
  check((await result(p, 'bot.chat.get')).canonical_session === null, "primary: its own Bot Chat is still none (reading others' never creates or adopts)")
  const asked = await result(p, 'bot.chat.ensure', { profile: 'x' })
  check(asked.created === true && asked.canonical_session.id !== idX, "primary: ensure ignores the profile param and acts for its own profile only")
  await p.stop()

  // The desktop's own create flow (session.create + session.title) registers its chat with `claim`; the first wins.
  const z = await start('z', shared)
  const mk = async () => (await result(z, 'session.create', { title: 'Bot Chat' })).session_id
  const made1 = await mk()
  await result(z, 'session.title', { session_id: made1, title: 'Bot Chat' })
  const claimed = await result(z, 'bot.chat.claim', { session_id: made1 })
  check(claimed.claimed === true && claimed.canonical_session.id === made1, 'claim: the desktop-made chat becomes the registered one')
  const made2 = await mk()
  await result(z, 'session.title', { session_id: made2, title: 'Bot Chat' })
  const lost = await result(z, 'bot.chat.claim', { session_id: made2 })
  check(lost.claimed === false && lost.canonical_session.id === made1, 'claim: a second chat does not displace the registered one')
  check((await result(z, 'bot.chat.ensure')).canonical_session.id === made1 && (await result(z, 'bot.chat.ensure')).created === false, 'ensure after claim returns the claimed chat')
  await x.stop()
  await y.stop()
  await z.stop()
} catch (error) {
  console.error(error)
  failures++
} finally {
  fs.rmSync(scratch, { recursive: true, force: true })
}
console.log(failures ? `\n${failures} check(s) failed` : '\nall checks passed')
process.exit(failures ? 1 : 0)
