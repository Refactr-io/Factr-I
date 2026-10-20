#!/usr/bin/env node
// Real profile/cron HTTP and RPC surfaces, with a local deterministic model and fresh homes.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const work = fileURLToPath(new URL('../../../../', import.meta.url))
const root = fs.mkdtempSync(path.join(process.env.SCRATCH || os.tmpdir(), 'bots-schedules-'))
const cfg = path.join(root, 'config'), home = path.join(root, 'engine')
const calls = [], engines = new Set()
let courier
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
async function until(test, timeout = 30000) {
  const deadline = Date.now() + timeout
  while (!await test()) { assert.ok(Date.now() < deadline, 'condition timed out'); await sleep(100) }
}
const model = http.createServer(async (req, res) => {
  let raw = ''; for await (const data of req) raw += data
  res.setHeader('content-type', 'application/json')
  if (req.method === 'GET') return res.end(JSON.stringify({ data: [{ id: 'schedule-stub' }] }))
  const body = JSON.parse(raw), text = 'SCHEDULE_RESULT'
  calls.push(body)
  const message = { role: 'assistant', content: text }
  if (!body.stream) return res.end(JSON.stringify({ id: 's', choices: [{ message, finish_reason: 'stop' }] }))
  res.setHeader('content-type', 'text/event-stream')
  for (const [delta, finish_reason] of [[message, null], [{}, 'stop']]) {
    res.write(`data: ${JSON.stringify({ id: 's', object: 'chat.completion.chunk', model: 'schedule-stub', choices: [{ index: 0, delta, finish_reason }] })}\n\n`)
  }
  res.end('data: [DONE]\n\n')
})
await new Promise(resolve => model.listen(0, '127.0.0.1', resolve))
const endpoint = `http://127.0.0.1:${model.address().port}/v1`
for (const dir of [cfg, home]) fs.mkdirSync(dir, { recursive: true })
fs.writeFileSync(path.join(cfg, 'config.yaml'), `model:\n  provider: custom\n  default: schedule-stub\n  base_url: ${endpoint}\n  context_length: 32768\n`)
fs.writeFileSync(path.join(home, 'config.toml'), `[provider]\ndefault_provider="local"\n[providers.local]\ntype="openai-compatible"\nbase_url="${endpoint}"\napi_key="none"\nrequires_api_key=false\ndefault_model="schedule-stub"\n[[providers.local.models]]\nid="schedule-stub"\ncontext_window=32768\n`)

async function start(profile) {
  const token = crypto.randomBytes(24).toString('hex')
  const userHome = path.join(root, 'homes', profile)
  fs.mkdirSync(userHome, { recursive: true })
  const child = spawn(process.env.FACTR_BIN || path.join(work, 'target/debug/factr'), ['--profile', profile, 'serve', '--host', '127.0.0.1', '--port', '0'], {
    cwd: userHome, env: { ...process.env, HOME: userHome, FACTR_HOME: home, FACTR_CONFIG_HOME: cfg,
      FACTR_RUNTIME_DIR: fs.mkdtempSync('/private/tmp/bots-schedule-rpc-'), FACTR_DASHBOARD_SESSION_TOKEN: token,
      OLLAMA_HOST: '127.0.0.1:1', FACTR_FEATURE_IDLE_MS: '300000' }, stdio: ['ignore', 'pipe', 'pipe']
  })
  engines.add(child)
  let stdout = '', stderr = ''
  child.stdout.on('data', data => { stdout += data })
  child.stderr.on('data', data => { stderr += data })
  await until(() => /FACTR_BACKEND_READY port=\d+/.test(stdout) || child.exitCode !== null)
  assert.equal(child.exitCode, null, stderr)
  const port = Number(stdout.match(/FACTR_BACKEND_READY port=(\d+)/)[1])
  const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`)
  const pending = new Map(); let next = 0
  ws.addEventListener('message', event => { const value = JSON.parse(String(event.data)); pending.get(value.id)?.(value) })
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve); ws.addEventListener('error', reject) })
  return {
    async rpc(method, params = {}) {
      const id = ++next
      const reply = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out\n${stderr.slice(-1500)}`)) }, 60000)
        pending.set(id, value => { clearTimeout(timer); pending.delete(id); resolve(value) })
        ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
      })
      assert.ok(!reply.error, JSON.stringify(reply.error))
      return reply.result
    },
    async api(route, method = 'GET', body) {
      const url = new URL(route, `http://127.0.0.1:${port}`)
      url.searchParams.set('feature', '1')
      const response = await fetch(url, { method, headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) })
      const value = await response.json()
      assert.equal(response.status, 200, JSON.stringify(value))
      return value
    },
    async stop() {
      ws.close(); child.kill('SIGTERM')
      await until(() => child.exitCode !== null || child.signalCode !== null, 10000)
      engines.delete(child)
    }
  }
}

try {
  const primary = await start('default')
  const fresh = await primary.rpc('profiles.create', { name: 'alpha', description: 'Read team documents', no_alias: true, no_skills: true, mirror_credentials: false, soul: 'Use the team vocabulary.' })
  assert.equal(fresh.ok, true)
  await primary.rpc('profiles.configure', { name: 'alpha', ui_meta: { 'factr-bots': { title: 'Alpha' } } })
  const described = await primary.rpc('profiles.describe', { name: 'alpha' })
  assert.equal(described.description, 'Read team documents')
  await primary.rpc('profiles.create', { name: 'alpha-copy', clone_from: 'alpha', no_alias: true, mirror_credentials: false })
  await primary.rpc('profiles.create', { name: 'beta', description: 'Process scheduled output', no_alias: true, no_skills: true, mirror_credentials: false })
  await primary.rpc('profiles.configure', { name: 'beta', ui_meta: { 'factr-bots': { title: 'Beta' } } })
  assert.equal(fs.readFileSync(path.join(cfg, 'profiles/alpha-copy/SOUL.md'), 'utf8'), 'Use the team vocabulary.')
  console.log('ok: real create, description, SOUL and clone surfaces')
  await primary.stop()

  let alpha = await start('alpha')
  const chat = (await alpha.rpc('bot.chat.ensure', { profile: 'alpha' })).canonical_session.id
  const created = await alpha.api('/api/cron/jobs', 'POST', { name: '[bot:alpha] Weekday check', schedule: '0 9 * * 1-5', prompt: 'SCHEDULE_LITERAL $(keep) `keep`', deliver: 'bot-chat', enabled_toolsets: ['file'] })
  assert.ok(created.id, JSON.stringify(created))
  const route = `/api/cron/jobs/${created.id}`
  await alpha.api(`${route}/pause`, 'POST')
  assert.equal((await alpha.api(route)).enabled, false)
  await alpha.stop()
  alpha = await start('alpha')
  assert.equal((await alpha.rpc('bot.chat.ensure', { profile: 'alpha' })).canonical_session.id, chat)
  assert.equal((await alpha.api(route)).enabled, false, 'paused schedule survives engine restart')
  await alpha.api(`${route}/resume`, 'POST')
  assert.equal((await alpha.api(route)).enabled, true)
  await alpha.api(`${route}/trigger`, 'POST')
  await until(async () => (await alpha.api(route)).last_status === 'ok', 60000)
  const history = await alpha.rpc('session.history', { session_id: chat })
  assert.ok(JSON.stringify(history).includes('SCHEDULE_RESULT'), 'scheduled output reaches the engine-owned Bot Chat')
  assert.ok(calls.some(body => JSON.stringify(body.messages).includes('[Cronjob')), 'Bot Chat processes the scheduled output')
  const scheduled = calls.find(body => JSON.stringify(body.messages).includes('SCHEDULE_LITERAL'))
  assert.ok(scheduled, 'scheduled instruction reached the real engine model')
  const tools = JSON.stringify(scheduled.tools)
  assert.ok(!/cronjob|heartbeat|session_goal/.test(tools), 'cron cannot offer self-scheduling tools')
  assert.ok(fs.readdirSync(path.join(cfg, 'profiles/alpha/cron/output'), { recursive: true }).some(name => String(name).endsWith('.md')))
  await alpha.api(route, 'DELETE')
  assert.ok(!JSON.stringify(await alpha.api('/api/cron/jobs')).includes(created.id))
  console.log('ok: routine create, pause, restart, resume, real fire, output, policy and delete')
  const beta = await start('beta')
  const betaChat = (await beta.rpc('bot.chat.ensure', { profile: 'beta' })).canonical_session.id
  const spool = path.join(cfg, 'bot_relay')
  fs.mkdirSync(path.join(spool, 'claimed'), { recursive: true })
  fs.mkdirSync(path.join(spool, 'outbox'), { recursive: true })
  let draining = false
  courier = setInterval(async () => {
    fs.writeFileSync(path.join(spool, 'desktop.json'), '{}')
    if (draining) return
    draining = true
    try {
      for (const file of fs.readdirSync(path.join(spool, 'outbox'))) {
        if (!file.endsWith('.json')) continue
        const envelope = JSON.parse(fs.readFileSync(path.join(spool, 'outbox', file), 'utf8'))
        fs.renameSync(path.join(spool, 'outbox', file), path.join(spool, 'claimed', file))
        assert.equal(envelope.target_profile, 'beta')
        let result = await beta.rpc('bot.chat.deliver', { profile: 'beta', envelope_id: envelope.id, text: envelope.message })
        while (result.status === 'pending') result = await beta.rpc('bot.chat.deliver_status', { envelope_id: envelope.id })
        assert.equal(result.ok, true, JSON.stringify(result))
        await alpha.rpc('bot_relay.reply', { id: envelope.id, reply: result.reply })
      }
    } catch (error) { console.error(error); process.exitCode = 1 }
    finally { draining = false }
  }, 100)
  await sleep(200)
  const cross = await alpha.api('/api/cron/jobs', 'POST', { name: '[bot:alpha] Send to Beta', schedule: '0 9 * * 1-5', prompt: 'SCHEDULE_TO_BETA', deliver: 'bot-chat:beta' })
  await alpha.api(`/api/cron/jobs/${cross.id}/trigger`, 'POST')
  await until(async () => (await alpha.api(`/api/cron/jobs/${cross.id}`)).last_status === 'ok', 60000)
  assert.ok(JSON.stringify(await beta.rpc('session.history', { session_id: betaChat })).includes('[Cronjob'))
  await alpha.api(`/api/cron/jobs/${cross.id}`, 'DELETE')
  clearInterval(courier)
  await beta.stop()
  console.log('ok: scheduled output reaches another profile through the engine courier')
  await alpha.stop()
  const cleanup = await start('default')
  await cleanup.api('/api/profiles/alpha-copy?feature=1', 'DELETE')
  assert.ok(!fs.existsSync(path.join(cfg, 'profiles/alpha-copy')))
  await cleanup.stop()
  console.log('PASS real profile lifecycle and scheduled engine turns')
} finally {
  clearInterval(courier)
  for (const child of engines) child.kill('SIGKILL')
  model.closeAllConnections(); model.close()
  console.log(`Isolated schedule artifacts: ${root}`)
}
