#!/usr/bin/env node
// Real-engine check of `bot.chat.deliver` / `bot.chat.deliver_status`: bot A's message becomes a real turn in bot B's
// own canonical Bot Chat, on B's own engine process, with no Python and no CLI subprocess. Two engines
// (`factr --profile a|b serve`) share ONE engine home (the packaged layout: one FACTR_HOME, one session store).
// The model is a scripted OpenAI-compatible server in this file, so every scenario is deterministic:
//   good turn (echo)   -> B's Bot Chat holds `Message from ... (@a): ...` and the bot's reply; the reply comes back
//   [SILENT]           -> an empty reply
//   HTTP 401           -> failure typed `provider_auth_or_access`, one request only (never retried)
//   HTTP 503 once      -> exactly one retry on the same session, no duplicate user message in the chat
//   hops > 2           -> refused with `hop_limit`; the engine refuses a message addressed to another profile
//   same envelope_id   -> one turn, the same answer
//   a window holds the chat, idle -> the delivery still runs headless and the window sees the turn live
//   a window is mid-turn on it     -> the delivery waits for that turn to end, then runs; the two never interleave
// Also: the teammate MCP server's tool schema is cached on first connection, so a fresh install's first chat could not
// load `message_agent` until the next start. `bot.chat.ensure` warms the cache first (checked with a stub MCP server).
// With REAL_MODEL=1 a third engine also answers one turn through local Ollama (factr/qwen3.8-27b:latest, 64k context).
//
//   cargo build --release --bin factr && node crates/factr-gateway/e2e/bot-chat-deliver.mjs
//   FACTR_BIN=<path to factr>  SCRATCH=<dir>  REAL_MODEL=1
import { spawn } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'

const BIN = process.env.FACTR_BIN || path.resolve('target/release/factr')
const scratch = fs.mkdtempSync(path.join(process.env.SCRATCH || os.tmpdir(), 'bot-chat-deliver-'))
const cfg = path.join(scratch, 'config')
const shared = path.join(scratch, 'shared-engine-home')
const MODEL = 'deliver-check:latest'
const engines = new Set()
let failures = 0
const check = (cond, msg) => {
  console.log(cond ? 'ok  ' : 'FAIL', msg)
  if (!cond) failures++
}
const sleep = ms => new Promise(r => setTimeout(r, ms))

// ── the scripted model ──────────────────────────────────────────────────────────────────────────────────────
const seen = [] // every chat request: { user, status }
const flaky = new Map() // marker -> requests seen
function userText(body) {
  const last = [...(body.messages || [])].reverse().find(m => m.role === 'user')
  const c = last?.content
  return typeof c === 'string' ? c : Array.isArray(c) ? c.map(p => p.text || '').join('') : ''
}
const model = http.createServer((req, res) => {
  let raw = ''
  req.on('data', d => (raw += d))
  req.on('end', () => {
    if (req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'application/json' })
      return res.end(JSON.stringify({ object: 'list', data: [{ id: MODEL, object: 'model' }] }))
    }
    const body = JSON.parse(raw || '{}')
    const user = userText(body)
    const delay = user.includes('SLOWTURN') ? 6000 : 0
    const entry = { user, status: 200, tools: (body.tools || []).map(t => t.function?.name || t.name) }
    seen.push(entry)
    const fail = (status, message, type) => {
      entry.status = status
      res.writeHead(status, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ error: { message, type, code: type } }))
    }
    if (user.includes('FAIL401')) return fail(401, 'Incorrect API key provided', 'authentication_error')
    if (user.includes('FAIL503ONCE')) {
      const n = (flaky.get('503') || 0) + 1
      flaky.set('503', n)
      if (n === 1) return fail(503, 'The server is overloaded', 'server_error')
    }
    const text = user.includes('SILENCE') ? '[SILENT]' : `ECHO: ${user}`
    const created = Math.floor(Date.now() / 1000)
    return setTimeout(() => respond(), delay)
    function respond() {
    if (body.stream) {
      res.writeHead(200, { 'content-type': 'text/event-stream' })
      const chunk = (delta, finish, usage) =>
        `data: ${JSON.stringify({ id: 'cmpl-1', object: 'chat.completion.chunk', created, model: MODEL, choices: [{ index: 0, delta, finish_reason: finish }], ...(usage ? { usage } : {}) })}\n\n`
      res.write(chunk({ role: 'assistant', content: text }, null))
      res.write(chunk({}, 'stop', { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 }))
      res.end('data: [DONE]\n\n')
    } else {
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ id: 'cmpl-1', object: 'chat.completion', created, model: MODEL, choices: [{ index: 0, message: { role: 'assistant', content: text }, finish_reason: 'stop' }], usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } }))
    }
    }
  })
})
await new Promise(r => model.listen(0, '127.0.0.1', r))
const modelUrl = `http://127.0.0.1:${model.address().port}/v1`

// ── engines ────────────────────────────────────────────────────────────────────────────────────────────────
function seed(bot, base, name, window = 8192, home = shared) {
  fs.mkdirSync(path.join(cfg, 'profiles', bot), { recursive: true })
  fs.mkdirSync(path.join(scratch, bot), { recursive: true })
  fs.mkdirSync(home, { recursive: true })
  fs.writeFileSync(
    path.join(home, 'config.toml'),
    `[provider]\ndefault_provider = "local"\n\n[providers.local]\ntype = "openai-compatible"\nbase_url = "${base}"\napi_key = "none"\nrequires_api_key = false\ndefault_model = "${name}"\n\n[[providers.local.models]]\nid = "${name}"\ncontext_window = ${window}\n`
  )
}

async function start(bot, modelName = MODEL, home = shared) {
  const token = crypto.randomBytes(24).toString('hex')
  const runtime = fs.mkdtempSync('/tmp/bcd-')
  const env = { ...process.env, HOME: path.join(scratch, bot), FACTR_HOME: home, FACTR_CONFIG_HOME: cfg, FACTR_RUNTIME_DIR: runtime, FACTR_DASHBOARD_SESSION_TOKEN: token, OLLAMA_HOST: '127.0.0.1:1' }
  const child = spawn(BIN, ['--profile', bot, '--provider', 'openai-compatible', '--model', modelName, 'serve', '--host', '127.0.0.1', '--port', '0'], { env, cwd: path.join(scratch, bot), stdio: ['ignore', 'pipe', 'pipe'] })
  engines.add(child)
  child.once('exit', () => engines.delete(child))
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
  const open = async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`)
    const pending = new Map()
    const events = []
    let next = 1
    ws.addEventListener('message', m => {
      const f = JSON.parse(String(m.data))
      if (f.id !== undefined && pending.has(f.id)) { pending.get(f.id)(f); pending.delete(f.id) }
      else if (f.method === 'event') events.push(f.params)
    })
    await new Promise((resolve, reject) => { ws.addEventListener('open', resolve); ws.addEventListener('error', reject) })
    const rpc = (method, params = {}) => {
      const id = `e${next++}`
      ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${bot}: ${method} timed out`)) }, 130_000)
        pending.set(id, frame => { clearTimeout(timer); resolve(frame) })
      })
    }
    return { rpc, events, close: () => ws.close() }
  }
  const win = await open()
  const stop = async () => {
    win.close()
    child.kill('SIGTERM')
    await new Promise(resolve => { child.on('exit', resolve); setTimeout(resolve, 5000) })
  }
  return { ...win, open, stop, port, stderr: () => stderr }
}

const ok = async (eng, method, params) => {
  const reply = await eng.rpc(method, params)
  if (reply.error) throw new Error(`${method}: ${JSON.stringify(reply.error)}`)
  return reply.result
}

/** deliver, then poll deliver_status until done (what the relay does). */
async function deliver(eng, params, { maxMs = 300_000 } = {}) {
  let res = await ok(eng, 'bot.chat.deliver', params)
  const t0 = Date.now()
  while (res.status === 'pending' && Date.now() - t0 < maxMs) res = await ok(eng, 'bot.chat.deliver_status', { envelope_id: params.envelope_id })
  return res
}

const id = () => crypto.randomBytes(8).toString('hex')
const attributed = body => `Message from 🤖 Alpha (@a): ${body}`
// Reading a chat attaches the reader as its live owner, so a short-lived connection does it and lets go.
async function transcript(eng, session) {
  const reader = await eng.open()
  try {
    return (await ok(reader, 'session.resume', { session_id: session })).messages || []
  } finally {
    reader.close()
    await sleep(300)
  }
}
async function ensureChat(eng, profile) {
  const maker = await eng.open() // the chat is minted on its own connection and let go, so no window holds it
  try {
    return (await ok(maker, 'bot.chat.ensure', { profile })).canonical_session.id
  } finally {
    maker.close()
    await sleep(300)
  }
}
const textOf = m => (typeof m.text === 'string' ? m.text : typeof m.content === 'string' ? m.content : JSON.stringify(m.content ?? m))

// A stub stdio MCP server exposing `message_agent`, standing in for tools/message_agent_mcp.py.
const stub = path.join(scratch, 'teammates-stub.mjs')
fs.writeFileSync(
  stub,
  `import readline from 'node:readline'
readline.createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line)
  if (m.id === undefined) return
  const result = m.method === 'initialize' ? { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'factr_teammates', version: '1' } }
    : m.method === 'tools/list' ? { tools: [{ name: 'message_agent', description: 'Message a teammate', inputSchema: { type: 'object', properties: { target: { type: 'string' }, message: { type: 'string' } }, required: ['target', 'message'] } }] }
    : {}
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\\n')
})
`
)

try {
  seed('a', modelUrl, MODEL)
  seed('b', modelUrl, MODEL)
  fs.writeFileSync(path.join(cfg, 'profiles', 'b', 'config.yaml'), `mcp_servers:\n  factr_teammates:\n    command: ${process.execPath}\n    args: ['${stub}']\n`)
  const a = await start('a')
  const b = await start('b')
  const cachePath = path.join(shared, 'mcp-schema-cache.json')
  check(!fs.existsSync(cachePath), 'a fresh install: no MCP schema cache yet')
  const chatB = await ensureChat(b, 'b')
  const cache = fs.existsSync(cachePath) ? JSON.parse(fs.readFileSync(cachePath, 'utf8')) : {}
  check(cache.servers?.factr_teammates?.tools?.some(t => t.name === 'message_agent'), "bot.chat.ensure cached the teammate server's tool schema before the chat's first spawn (message_agent is available at once)")

  // The real fresh-install condition: a brand-new store, a bot created, and the desktop makes the bot's FIRST chat with a
  // plain session.create (+ claim), never `ensure`. The first request the model gets must already offer message_agent.
  const freshHome = path.join(scratch, 'fresh-engine-home')
  seed('f', modelUrl, MODEL, 8192, freshHome)
  const python = process.env.FACTR_BACKEND_PYTHON
  const backend = process.env.FACTR_BACKEND_PYTHONPATH
  fs.writeFileSync(path.join(cfg, 'profiles', 'f', 'profile.yaml'), 'ui_meta:\n  factr-bots:\n    title: Fresh Bot\n')
  fs.writeFileSync(path.join(cfg, 'profiles', 'f', 'config.yaml'), python && backend
    ? `mcp_servers:\n  factr_teammates:\n    command: ${JSON.stringify(python)}\n    args: ['-m', 'tools.message_agent_mcp']\n    env:\n      PYTHONPATH: ${JSON.stringify(backend)}\n    timeout_secs: 900\n`
    : `mcp_servers:\n  factr_teammates:\n    command: ${process.execPath}\n    args: ['${stub}']\n`)
  const f = await start('f', MODEL, freshHome)
  check(!fs.existsSync(path.join(freshHome, 'mcp-schema-cache.json')), 'fresh store: nothing cached before the first chat')
  const firstChat = await ok(f, 'session.create', { title: 'Bot Chat', hidden: true, follow_profile_config: true })
  await ok(f, 'session.title', { session_id: firstChat.session_id, title: 'Bot Chat' })
  await ok(f, 'bot.chat.claim', { profile: 'f', session_id: firstChat.session_id })
  // New Bot mounts its fresh chat before submitting the intro on the creation connection.
  const firstWindow = await f.open()
  await ok(firstWindow, 'session.resume', { session_id: firstChat.session_id })
  await ok(f, 'prompt.submit', { session_id: firstChat.session_id, text: 'first words in a fresh install' })
  for (let i = 0; i < 100 && !seen.some(x => x.user.includes('first words in a fresh install')); i++) await sleep(100)
  const firstRequest = seen.find(x => x.user.includes('first words in a fresh install'))
  check(firstRequest?.tools?.some(t => /message_agent/.test(t)), `the bot's very first session offers message_agent (tools: ${(firstRequest?.tools || []).join(', ').slice(0, 200)})`)
  check(firstWindow.events.some(e => e.type === 'message.complete' && e.session_id === firstChat.session_id), 'the mounted window sees the creation-time intro complete')
  firstWindow.close()
  await f.stop()
  const chatA = await ensureChat(a, 'a')
  check(chatA !== chatB, 'two engines, two Bot Chats, one shared store')

  // A good turn: the message lands in B's own Bot Chat with its attribution, the model answers, the reply returns.
  const good = await deliver(b, { profile: 'b', text: attributed('what is two plus two'), from_handle: 'a', envelope_id: id(), hops: 1 })
  check(good.status === 'done' && good.ok === true, `deliver finishes ok (${JSON.stringify(good).slice(0, 160)})`)
  check(good.reply === `ECHO: ${attributed('what is two plus two')}`, 'the reply is the bot\'s answer, and the model saw the attribution prefix exactly')
  check(good.session_id === chatB, "the turn ran in B's canonical Bot Chat")
  const msgs = await transcript(b, chatB)
  check(msgs.some(m => m.role === 'user' && textOf(m).includes('Message from 🤖 Alpha (@a): what is two plus two')), "B's Bot Chat shows the incoming 'Message from ... (@a):' message")
  check(msgs.some(m => m.role === 'assistant' && textOf(m).includes('ECHO:')), "B's Bot Chat shows the bot's reply")
  check((await transcript(a, chatA)).length === 0, "A's own Bot Chat is untouched")
  const sessionsB = (await ok(b, 'session.list', { limit: 200, include_hidden: true })).sessions
  check(sessionsB.filter(s => s.title === 'Bot Chat').length === 2, 'the shared store holds exactly the two Bot Chats (a and b); no extra session per message')

  // The same envelope again: the same answer, no second turn.
  const turnsBefore = seen.filter(s => s.user.includes('what is two plus two')).length
  const envelope = id()
  const first = await deliver(b, { profile: 'b', text: attributed('idempotent please'), from_handle: 'a', envelope_id: envelope, hops: 1 })
  const again = await deliver(b, { profile: 'b', text: attributed('idempotent please'), from_handle: 'a', envelope_id: envelope, hops: 1 })
  check(first.reply === again.reply && again.status === 'done', 'a repeated envelope_id returns the stored result')
  check(seen.filter(s => s.user.includes('idempotent please')).length === 1, 'a repeated envelope_id runs one turn, not two')
  check(turnsBefore === 1, 'and the earlier message ran exactly once')

  // [SILENT]: nothing to send back.
  const silent = await deliver(b, { profile: 'b', text: attributed('SILENCE'), from_handle: 'a', envelope_id: id(), hops: 1 })
  check(silent.status === 'done' && silent.ok === true && silent.reply === '', '[SILENT] gives an empty reply')

  // An auth failure is typed and not retried.
  const before401 = seen.length
  const auth = await deliver(b, { profile: 'b', text: attributed('FAIL401'), from_handle: 'a', envelope_id: id(), hops: 1 })
  check(auth.status === 'done' && auth.ok === false && auth.reason === 'provider_auth_or_access', `an auth failure carries reason provider_auth_or_access (${auth.reason}: ${String(auth.error).slice(0, 120)})`)
  check(seen.slice(before401).filter(s => s.user.includes('FAIL401')).length === 1, 'an auth failure is never retried (one model request)')

  // A transient failure retries once on the same session, and the chat holds the message once.
  const retried = await deliver(b, { profile: 'b', text: attributed('FAIL503ONCE'), from_handle: 'a', envelope_id: id(), hops: 1 })
  check(retried.status === 'done' && retried.ok === true && retried.reply.includes('FAIL503ONCE'), `a transient failure retries once and succeeds (${JSON.stringify(retried).slice(0, 140)})`)
  const after = await transcript(b, chatB)
  check(after.filter(m => m.role === 'user' && textOf(m).includes('FAIL503ONCE')).length === 1, 'the retry resent the same message: it is in the chat exactly once')

  // The hop cap, and the wrong-engine guard.
  const hops3 = await deliver(b, { profile: 'b', text: attributed('too deep'), from_handle: 'a', envelope_id: id(), hops: 3 })
  check(hops3.status === 'done' && hops3.ok === false && hops3.reason === 'hop_limit', 'a message already 3 hops deep is refused with hop_limit')
  check(!seen.some(s => s.user.includes('too deep')), '... and never reaches the model')
  const hops2 = await deliver(b, { profile: 'b', text: attributed('two hops is fine'), from_handle: 'a', envelope_id: id(), hops: 2 })
  check(hops2.ok === true, 'a message at the cap (2 hops) is delivered')
  const wrong = await b.rpc('bot.chat.deliver', { profile: 'a', text: 'misrouted', envelope_id: id() })
  check(wrong.error && /serves @b/.test(wrong.error.message), "an engine refuses a message addressed to another profile")
  const missing = await b.rpc('bot.chat.deliver_status', { envelope_id: 'nope' })
  check(missing.error, 'deliver_status for an unknown envelope is an error')

  // The 16,000 character cap on the reply (the model echoes, so a long message makes a long reply).
  const long = await deliver(b, { profile: 'b', text: attributed('x'.repeat(16_000)), from_handle: 'a', envelope_id: id(), hops: 1 })
  check(long.ok === true && long.reply.length <= 16_100 && long.reply.includes('[reply cut at 16000 of'), 'a reply over 16,000 characters is cut with a note')
  const huge = await deliver(b, { profile: 'b', text: attributed('y'.repeat(16_300)), from_handle: 'a', envelope_id: id(), hops: 1 })
  check(huge.ok === false && /too long/.test(huge.error), 'a message over the cap is refused before any turn')

  // A window that holds the chat but is idle does not stop a delivery: the turn still runs, and the window sees it.
  const window = await b.open()
  await ok(window, 'session.resume', { session_id: chatB })
  const idleText = attributed('hello idle window')
  const idleRun = await deliver(b, { profile: 'b', text: idleText, from_handle: 'a', envelope_id: id(), hops: 1 })
  check(idleRun.status === 'done' && idleRun.ok === true && idleRun.reply === `ECHO: ${idleText}`, `an idle window holding the chat does not block delivery (${JSON.stringify(idleRun).slice(0, 120)})`)
  await sleep(300)
  check(window.events.some(e => e.session_id === chatB && e.type === 'message.complete'), 'the window sees the delivered turn complete live in its chat')

  // The window is mid-turn: a prompt sent now would not queue (the engine steers a running turn), so the delivery waits for
  // the window's turn to end, then runs. The two turns never interleave and both are in the chat, in order.
  await ok(window, 'prompt.submit', { session_id: chatB, text: 'SLOWTURN user is typing' })
  await sleep(500)
  const busyText = attributed('hello busy window')
  const t0 = Date.now()
  const waited = await deliver(b, { profile: 'b', text: busyText, from_handle: 'a', envelope_id: id(), hops: 1 })
  check(waited.status === 'done' && waited.ok === true && waited.reply === `ECHO: ${busyText}`, `a delivery to a chat mid-turn waits, then returns its own reply (${JSON.stringify(waited).slice(0, 140)})`)
  check(Date.now() - t0 >= 4000, `... it really waited for the window's turn (${Date.now() - t0} ms)`)
  const order = seen.map(x => x.user)
  check(order.findIndex(u => u.includes('SLOWTURN')) >= 0 && order.findIndex(u => u.includes('hello busy window')) > order.findIndex(u => u.includes('SLOWTURN')), "the window's turn reached the model first, the delivered one after it")
  window.close()
  await sleep(300)
  const both = await transcript(b, chatB)
  const idx = t => both.findIndex(m => m.role === 'user' && textOf(m).includes(t))
  check(idx('SLOWTURN user is typing') >= 0 && idx('hello busy window') > idx('SLOWTURN user is typing'), "B's Bot Chat holds the window's turn and then the delivered one, in order")

  // A delivery between two engines does not disturb the registry: still one Bot Chat each after a restart of B.
  await b.stop()
  const b2 = await start('b')
  check((await ok(b2, 'bot.chat.get', { profile: 'b' })).canonical_session?.id === chatB, 'after B restarts its Bot Chat is the same one, history intact')
  check((await transcript(b2, chatB)).filter(m => m.role === 'user' && textOf(m).includes('Message from 🤖 Alpha')).length >= 4, 'the delivered messages survived the restart')
  await b2.stop()
  await a.stop()

  if (process.env.REAL_MODEL === '1') {
    const REAL = process.env.REAL_MODEL_NAME || 'factr/qwen3.8-27b:latest'
    seed('c', 'http://127.0.0.1:11434/v1', REAL, 65536)
    const c = await start('c', REAL)
    const real = await deliver(c, { profile: 'c', text: attributed('Reply with exactly the single word PONG and nothing else.'), from_handle: 'a', envelope_id: id(), hops: 1 }, { maxMs: 600_000 })
    check(real.ok === true && /PONG/i.test(real.reply), `local Ollama (${REAL}) answers a delivered message (${JSON.stringify(real).slice(0, 160)})`)
    await c.stop()
  }
} catch (e) {
  console.error('ERROR', e)
  failures++
} finally {
  for (const child of engines) child.kill('SIGTERM')
  await Promise.all([...engines].map(child => new Promise(resolve => {
    child.once('exit', resolve)
    setTimeout(() => { child.kill('SIGKILL'); resolve() }, 5000).unref()
  })))
  model.close()
  fs.rmSync(scratch, { recursive: true, force: true })
}
console.log(failures ? `\n${failures} check(s) FAILED` : '\nall checks passed')
process.exit(failures ? 1 : 0)
