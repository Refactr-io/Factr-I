#!/usr/bin/env node
// The whole `message_agent` chain with a real engine at the end: bot A's message_agent tool (the Python MCP server the
// engine runs for it) queues an envelope, the Desktop relay (played here) drains it and calls `bot.chat.deliver` on bot
// B's own engine, B's engine runs the turn in B's Bot Chat, and A's tool call returns B's reply. No Python CLI turn
// runs for B. B's model is a scripted OpenAI-compatible server, so every case is deterministic:
//   good reply  -> A's tool result is "Reply from @b: ECHO: Message from 🤖 a (@a): ..." and B's Bot Chat holds both
//   auth error  -> A's tool result is "Delivery to @b failed [reason: provider_auth_or_access]: ..."
//   [SILENT]    -> A's tool result is the empty reply
//   no relay    -> without the Desktop heartbeat the message falls back to the local CLI turn (nothing is queued)
//   hop cap     -> a turn already two hops deep cannot message on: `hop_limit`
//
//   cargo build --release --bin factr && node crates/factr-gateway/e2e/bot-message-agent.mjs
//   FACTR_BIN=<path to factr>  SCRATCH=<dir>  BACKEND=<repo>/backend  PYTHON=<python with the backend installed>
import { spawn, spawnSync } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'

const BIN = process.env.FACTR_BIN || path.resolve('target/release/factr')
const BACKEND = process.env.BACKEND || path.resolve('../backend')
const PYTHON = process.env.PYTHON || path.join(BACKEND, '.venv/bin/python')
const scratch = fs.mkdtempSync(path.join(process.env.SCRATCH || os.tmpdir(), 'bot-message-agent-'))
const cfg = path.join(scratch, 'config') // the install root: profiles/<bot> under it, bot_relay/ beside them
const shared = path.join(scratch, 'engine-home')
const MODEL = 'agent-check:latest'
let failures = 0
const check = (cond, msg) => {
  console.log(cond ? 'ok  ' : 'FAIL', msg)
  if (!cond) failures++
}
const sleep = ms => new Promise(r => setTimeout(r, ms))

// ── B's scripted model ─────────────────────────────────────────────────────────────────────────────────────
const seen = []
const model = http.createServer((req, res) => {
  let raw = ''
  req.on('data', d => (raw += d))
  req.on('end', () => {
    if (req.method === 'GET') {
      res.writeHead(200, { 'content-type': 'application/json' })
      return res.end(JSON.stringify({ object: 'list', data: [{ id: MODEL, object: 'model' }] }))
    }
    const body = JSON.parse(raw || '{}')
    const last = [...(body.messages || [])].reverse().find(m => m.role === 'user' && !JSON.stringify(m.content).includes('Teammates and roles:'))
    const user = typeof last?.content === 'string' ? last.content : (last?.content || []).map(p => p.text || '').join('')
    seen.push(user)
    if (user.includes('FAIL401')) {
      res.writeHead(401, { 'content-type': 'application/json' })
      return res.end(JSON.stringify({ error: { message: 'Incorrect API key provided', type: 'authentication_error' } }))
    }
    const text = user.includes('SILENCE') ? '[SILENT]' : `ECHO: ${user}`
    res.writeHead(200, { 'content-type': 'text/event-stream' })
    const chunk = (delta, finish, usage) => `data: ${JSON.stringify({ id: 'c', object: 'chat.completion.chunk', created: 1, model: MODEL, choices: [{ index: 0, delta, finish_reason: finish }], ...(usage ? { usage } : {}) })}\n\n`
    res.write(chunk({ role: 'assistant', content: text }, null))
    res.write(chunk({}, 'stop', { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 }))
    res.end('data: [DONE]\n\n')
  })
})
await new Promise(r => model.listen(0, '127.0.0.1', r))

// ── the install: two bots, B served by its own engine ─────────────────────────────────────────────────────
for (const bot of ['a', 'b']) {
  const dir = path.join(cfg, 'profiles', bot)
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(path.join(dir, 'profile.yaml'), `description: ${bot}\nui_meta:\n  factr-bots:\n    title: ${bot}\n`)
}
fs.mkdirSync(shared, { recursive: true })
fs.writeFileSync(
  path.join(shared, 'config.toml'),
  `[provider]\ndefault_provider = "local"\n\n[providers.local]\ntype = "openai-compatible"\nbase_url = "http://127.0.0.1:${model.address().port}/v1"\napi_key = "none"\nrequires_api_key = false\ndefault_model = "${MODEL}"\n\n[[providers.local.models]]\nid = "${MODEL}"\ncontext_window = 8192\n`
)

async function startEngine(bot) {
  const token = crypto.randomBytes(24).toString('hex')
  const env = { ...process.env, HOME: path.join(scratch, bot), FACTR_HOME: shared, FACTR_CONFIG_HOME: cfg, FACTR_RUNTIME_DIR: fs.mkdtempSync('/tmp/bma-'), FACTR_DASHBOARD_SESSION_TOKEN: token, OLLAMA_HOST: '127.0.0.1:1' }
  fs.mkdirSync(env.HOME, { recursive: true })
  const child = spawn(BIN, ['--profile', bot, '--provider', 'openai-compatible', '--model', MODEL, 'serve', '--host', '127.0.0.1', '--port', '0'], { env, cwd: env.HOME, stdio: ['ignore', 'pipe', 'pipe'] })
  const port = await new Promise((resolve, reject) => {
    let buf = ''
    child.stdout.on('data', d => { buf += d; const m = buf.match(/FACTR_BACKEND_READY port=(\d+)/); if (m) resolve(Number(m[1])) })
    child.on('exit', code => reject(new Error(`engine ${bot} exited ${code}`)))
    setTimeout(() => reject(new Error('engine did not start')), 120_000)
  })
  const open = async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`)
    const pending = new Map()
    let n = 1
    ws.addEventListener('message', m => { const f = JSON.parse(String(m.data)); if (f.id !== undefined && pending.has(f.id)) { pending.get(f.id)(f); pending.delete(f.id) } })
    await new Promise((resolve, reject) => { ws.addEventListener('open', resolve); ws.addEventListener('error', reject) })
    return { rpc: (method, params = {}) => { const id = `e${n++}`; ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params })); return new Promise(resolve => pending.set(id, resolve)) }, close: () => ws.close() }
  }
  return { child, open }
}

const py = (code, env = {}) => {
  const r = spawnSync(PYTHON, ['-c', code], { env: { ...process.env, PYTHONPATH: BACKEND, FACTR_CONFIG_HOME: cfg, ...env }, encoding: 'utf8' })
  if (r.status !== 0) throw new Error(`python failed: ${r.stderr}`)
  return r.stdout
}

// ── the Desktop relay: heartbeat, drain, deliver on B's engine, write the reply back ────────────────────────
let beat = true
let engineB
const relay = setInterval(async () => {
  const dir = path.join(cfg, 'bot_relay')
  fs.mkdirSync(path.join(dir, 'outbox'), { recursive: true })
  if (beat) fs.writeFileSync(path.join(dir, 'desktop.json'), '{}')
  if (!engineB) return
  let envelopes = []
  try {
    envelopes = JSON.parse(py('import json; from tools.bot_relay import claim_pending_envelopes; print(json.dumps(claim_pending_envelopes(__import__("os").environ["FACTR_CONFIG_HOME"])))') || '[]')
  } catch { return }
  for (const env of envelopes) void (async () => {
    const b = await engineB.open()
    const call = async (method, params) => { const r = await b.rpc(method, params); if (r.error) throw new Error(r.error.message); return r.result }
    let res = await call('bot.chat.deliver', { profile: env.target_profile, text: env.message, from_handle: env.from_handle, envelope_id: env.id, hops: env.hops })
    while (res.status === 'pending') res = await call('bot.chat.deliver_status', { envelope_id: env.id })
    b.close()
    const reply = res.ok ? { reply: res.reply } : { error: res.error, reason: res.reason }
    py(`import json,os; from tools.bot_relay import write_reply; d=json.loads(os.environ["REPLY"]); write_reply(os.environ["FACTR_CONFIG_HOME"], os.environ["ENVELOPE"], reply=d.get("reply",""), error=d.get("error",""), reason=d.get("reason",""))`, { REPLY: JSON.stringify(reply), ENVELOPE: env.id })
  })()
}, 500)

// ── A's message_agent tool: the MCP server the engine runs for it, driven over stdio ───────────────────────
function mcp(extraEnv = {}, profile = 'a') {
  const child = spawn(PYTHON, ['-m', 'tools.message_agent_mcp'], { env: { ...process.env, PYTHONPATH: BACKEND, FACTR_CONFIG_HOME: profile === 'default' ? cfg : path.join(cfg, 'profiles', profile), ...extraEnv }, stdio: ['pipe', 'pipe', 'pipe'] })
  const pending = new Map()
  let buf = ''
  child.stdout.on('data', d => {
    buf += d
    for (let i; (i = buf.indexOf('\n')) >= 0; ) {
      const line = buf.slice(0, i)
      buf = buf.slice(i + 1)
      const m = JSON.parse(line)
      if (pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id) }
    }
  })
  let n = 1
  const call = (name, args) => new Promise(resolve => {
    const id = n++
    pending.set(id, resolve)
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method: 'tools/call', params: { name, arguments: args } }) + '\n')
  })
  return { call, close: () => child.kill('SIGTERM') }
}
const text = r => r.result.content[0].text

try {
  engineB = await startEngine('b')
  const maker = await engineB.open()
  const chatB = (await maker.rpc('bot.chat.ensure', { profile: 'b' })).result.canonical_session.id
  maker.close()
  await sleep(300)
  const transcript = async () => {
    const reader = await engineB.open()
    const msgs = (await reader.rpc('session.resume', { session_id: chatB })).result.messages || []
    reader.close()
    await sleep(200)
    return msgs
  }
  const tool = mcp()

  const good = await tool.call('message_agent', { target: 'b', message: 'ping from a' })
  check(good.result.isError === false && /Reply from @b/.test(text(good)), `A's message_agent returns B's reply (${text(good).replace(/\n/g, ' | ').slice(0, 200)})`)
  check(/ECHO: Message from 🤖 a \(@a\): ping from a/.test(text(good)), "the reply is B's engine's answer to the attributed message")
  check(seen.some(u => u.includes('Message from 🤖 a (@a): ping from a')), 'B\'s model saw `Message from 🤖 a (@a): ...` exactly as the CLI path composes it')
  const msgs = await transcript()
  const textOf = m => (typeof m.text === 'string' ? m.text : typeof m.content === 'string' ? m.content : JSON.stringify(m.content ?? m))
  check(msgs.some(m => m.role === 'user' && textOf(m).includes('Message from 🤖 a (@a): ping from a')) && msgs.some(m => m.role === 'assistant' && textOf(m).includes('ECHO:')), "B's Bot Chat shows the incoming message and the bot's reply")

  const primaryTool = mcp({}, 'default')
  try {
    const handoff = await primaryTool.call('message_agent', { target: 'b', message: 'handoff from the user chat' })
    check(handoff.result.isError === false && /Reply from @b/.test(text(handoff)), 'the default chat can send to a bot through the live relay and get its reply')
    check(seen.some(u => u.includes('(@factr): handoff from the user chat')), 'the bot receives default-chat sender attribution')
  } finally {
    primaryTool.close()
  }

  const auth = await tool.call('message_agent', { target: 'b', message: 'FAIL401 please' })
  check(auth.result.isError === true || /failed/.test(text(auth)), `an auth failure reaches A (${text(auth).slice(0, 160)})`)
  check(/\[reason: provider_auth_or_access\]/.test(text(auth)), 'and carries the typed [reason: provider_auth_or_access] tag')

  const silent = await tool.call('message_agent', { target: 'b', message: 'SILENCE' })
  check(/\(empty reply\)/.test(text(silent)) || text(silent).trim().endsWith(':') || /Reply from @b:\s*$/.test(text(silent).trim()), `[SILENT] gives A an empty reply (${JSON.stringify(text(silent)).slice(0, 100)})`)

  // The hop cap: a turn already two relays deep (FACTR_RELAY_HOPS, or the marker the engine writes) cannot message on.
  const deep = mcp({ FACTR_RELAY_HOPS: '2' })
  const capped = await deep.call('message_agent', { target: 'b', message: 'one hop too many' })
  check(/hop_limit/.test(text(capped)), `a turn two hops deep cannot relay on (${text(capped).slice(0, 120)})`)
  check(!seen.some(u => u.includes('one hop too many')), '... and the message never reaches B')
  deep.close()

  // No Desktop relay: nothing is queued, the local CLI turn is the fallback (it cannot run here: no CLI is configured).
  beat = false
  fs.rmSync(path.join(cfg, 'bot_relay', 'desktop.json'), { force: true })
  const before = fs.readdirSync(path.join(cfg, 'bot_relay', 'outbox')).length
  const lone = mcp()
  const fallback = lone.call('message_agent', { target: 'b', message: 'no relay running' })
  await sleep(2500)
  check(fs.readdirSync(path.join(cfg, 'bot_relay', 'outbox')).length === before, 'without the relay heartbeat nothing is queued for it')
  lone.close()
  void fallback
  tool.close()
  engineB.child.kill('SIGTERM')
} catch (e) {
  console.error('ERROR', e)
  failures++
} finally {
  clearInterval(relay)
  model.close()
  fs.rmSync(scratch, { recursive: true, force: true })
}
console.log(failures ? `\n${failures} check(s) FAILED` : '\nall checks passed')
process.exit(failures ? 1 : 0)
