// Production local-sandbox launcher + real engines, native tools, feature Python and a deterministic model.
// Run: FACTR_BIN=... desktop/node_modules/.bin/tsx engine/crates/factr-gateway/e2e/bot-local-sandbox.mts
import assert from 'node:assert/strict'
import { spawn, type ChildProcess } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { prepareBotSandbox } from '../../../../desktop/app/electron/bot-local-sandbox.ts'

const repo = fileURLToPath(new URL('../../../../', import.meta.url))
const base = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'bots-local-qa-')))
const cfg = path.join(base, 'config')
const binary = process.env.FACTR_BIN || path.join(repo, 'target/debug/factr')
const python = process.env.FACTR_BACKEND_PYTHON || path.join(repo, 'backend/.venv/bin/python')
const children = new Set<ChildProcess>()
const seen: any[] = []
let victim: ChildProcess | undefined
const canary = path.join(base, 'host-canary')
fs.writeFileSync(canary, 'HOST_ONLY_CANARY')
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
async function until(test: () => boolean | Promise<boolean>, ms = 30000) {
  const deadline = Date.now() + ms
  while (!await test()) { assert.ok(Date.now() < deadline, 'condition timed out'); await sleep(50) }
}
const model = http.createServer(async (req, res) => {
  let raw = ''; for await (const part of req) raw += part
  res.setHeader('content-type', 'application/json')
  if (req.method === 'GET') return res.end(JSON.stringify({ data: [{ id: 'sandbox-stub' }] }))
  const body = JSON.parse(raw), messages = body.messages || []
  seen.push(body)
  const user = [...messages].reverse().find(m => m.role === 'user' && !JSON.stringify(m.content).includes('Teammates and roles:'))
  const prompt = typeof user?.content === 'string' ? user.content : user?.content?.map(p => p.text || '').join('') || ''
  const result = [...messages].reverse().find(m => m.role === 'tool')
  const load = [...messages].some(m => m.tool_calls?.some(t => t.function.name === 'load_tools'))
  let name = 'bash', args: any = { command: 'echo OWN_FILE > own.txt; cat own.txt', timeout: 10000 }
  if (prompt.includes('DENY_SHELL')) args.command = `cat '${canary}' 2>/dev/null || echo SHELL_DENIED; (echo bad > '${canary}') 2>/dev/null || echo WRITE_DENIED; kill -TERM ${victim?.pid} 2>/dev/null || echo SIGNAL_DENIED`
  if (prompt.includes('DENY_READ')) { name = 'read'; args = { file_path: canary } }
  if (prompt.includes('DENY_WRITE')) { name = 'write'; args = { file_path: canary, content: 'overwrite' } }
  if (prompt.includes('LOCAL_HANDOFF')) { name = 'message_agent'; args = { target: 'beta', message: 'PING_FOR_BETA' } }
  const offered = body.tools?.some(t => t.function?.name === name)
  const tool = !offered && !load ? { name: 'load_tools', args: { names: [name] } } : { name, args }
  const toolDone = result && messages.some(m => m.tool_calls?.some(t => t.function.name === name))
  const schedule = /SCHEDULE_LOCAL|LOCAL_SCHEDULE_RESULT/.test(prompt)
  const message = schedule ? { role: 'assistant', content: 'LOCAL_SCHEDULE_RESULT' } : prompt.includes('PING_FOR_BETA') ? { role: 'assistant', content: 'BETA_SANDBOX_REPLY' } : toolDone ? { role: 'assistant', content: `RESULT ${result.content}` }
    : { role: 'assistant', tool_calls: [{ id: crypto.randomUUID(), type: 'function', function: { name: tool.name, arguments: JSON.stringify(tool.args) } }] }
  const finished = schedule || toolDone || prompt.includes('PING_FOR_BETA')
  if (!body.stream) return res.end(JSON.stringify({ id: 'sb', choices: [{ message, finish_reason: finished ? 'stop' : 'tool_calls' }] }))
  res.setHeader('content-type', 'text/event-stream')
  for (const [delta, finish_reason] of [[message, null], [{}, finished ? 'stop' : 'tool_calls']]) {
    if ((delta as any).tool_calls) (delta as any).tool_calls[0].index = 0
    res.write(`data: ${JSON.stringify({ id: 'sb', object: 'chat.completion.chunk', model: 'sandbox-stub', choices: [{ index: 0, delta, finish_reason }] })}\n\n`)
  }
  res.end('data: [DONE]\n\n')
})
await new Promise<void>(resolve => model.listen(0, '127.0.0.1', resolve))
const endpoint = `http://127.0.0.1:${(model.address() as any).port}/v1`
async function start(profile: string) {
  const root = path.join(cfg, 'profiles', profile)
  fs.mkdirSync(root, { recursive: true })
  fs.writeFileSync(path.join(root, 'config.yaml'), `model:\n  provider: custom\n  default: sandbox-stub\n  base_url: ${endpoint}\n  api_key: none\napprovals:\n  mode: 'off'\nauxiliary:\n  title_generation:\n    enabled: false\nplugins:\n  enabled: []\n`)
  fs.writeFileSync(path.join(root, 'profile.yaml'), `ui_meta:\n  factr-bots:\n    title: ${profile}\n`)
  const token = crypto.randomBytes(24).toString('hex')
  const launch = prepareBotSandbox({ profile, configHome: cfg, command: binary,
    args: ['--profile', profile, 'serve', '--host', '127.0.0.1', '--port', '0', '--token-stdin'],
    env: { ...process.env, FACTR_BACKEND_PYTHON: python, FACTR_BACKEND_PYTHONPATH: path.join(repo, 'backend'),
      FACTR_BACKEND_TOOLS_DIR: path.join(repo, 'backend/tools'), PYTHONDONTWRITEBYTECODE: '1', OLLAMA_HOST: '127.0.0.1:1' },
    readRoots: [binary, path.join(repo, 'backend'), path.dirname(path.dirname(fs.realpathSync(python)))] })
  const child = spawn(launch.command, launch.args, { cwd: launch.cwd, env: launch.env, stdio: ['pipe', 'pipe', 'pipe'] })
  children.add(child)
  let output = ''
  child.stdout.on('data', data => { output += data })
  child.stderr.on('data', data => { output += data })
  child.once('exit', () => { children.delete(child); launch.cleanup() })
  child.stdin.end(token + '\n')
  await until(() => /FACTR_BACKEND_READY port=\d+/.test(output) || child.exitCode !== null)
  assert.equal(child.exitCode, null, output)
  const port = Number(output.match(/FACTR_BACKEND_READY port=(\d+)/)![1])
  const api = async (route: string, body?: any, method = body ? 'POST' : 'GET') => {
    const url = new URL(route, `http://127.0.0.1:${port}`)
    url.searchParams.set('profile', profile)
    const response = await fetch(url, { method,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(60000) })
    assert.equal(response.status, 200, await response.clone().text() + '\n' + output)
    return response.json() as Promise<any>
  }
  const rpc = async (method: string, params: any = {}) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`)
    try {
      await new Promise<void>((resolve, reject) => { ws.onopen = () => resolve(); ws.onerror = reject })
      return await new Promise<any>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(method + ' timeout\n' + output)), 60000)
        ws.onmessage = event => {
          const frame = JSON.parse(String(event.data)); if (frame.id !== 1) return
          clearTimeout(timer); frame.error ? reject(new Error(JSON.stringify(frame.error))) : resolve(frame.result)
        }
        ws.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }))
      })
    } finally { ws.close() }
  }
  return { root, child, launch, api, rpc, output: () => output, stop: async () => {
    child.kill('SIGTERM'); await until(() => child.exitCode !== null || child.signalCode !== null); launch.cleanup()
  } }
}

try {
  const alpha = await start('alpha'), beta = await start('beta')
  const roster = [{ profile: 'beta', handle: 'beta', title: 'Beta', description: 'Research', connection_id: 'computer-beta', online: true }]
  assert.equal((await alpha.rpc('bot_relay.roster.sync', { agents: roster })).count, 1, 'cold Alpha accepts a later-created Beta without a Python drain')
  victim = beta.child
  const run = (prompt: string) => alpha.api('/api/agent/run', { prompt, surface: 'bot', title: 'Bot Chat', session_key: prompt, timeout_s: 30 })
  const canonical = (await alpha.rpc('bot.chat.ensure', { profile: 'alpha' })).canonical_session.id
  const own = await run('CHECK_OWN')
  assert.equal(own.ok, true, JSON.stringify(own) + alpha.output())
  assert.ok(JSON.stringify(own).includes('OWN_FILE'), JSON.stringify(own))
  assert.equal(fs.readFileSync(path.join(alpha.root, 'workspace/own.txt'), 'utf8'), 'OWN_FILE\n')
  const denied = await run('DENY_SHELL')
  assert.equal(denied.ok, true, JSON.stringify(denied))
  for (const marker of ['SHELL_DENIED', 'WRITE_DENIED', 'SIGNAL_DENIED']) assert.ok(JSON.stringify(denied).includes(marker), JSON.stringify(denied))
  for (const prompt of ['DENY_READ', 'DENY_WRITE']) {
    const value = await run(prompt)
    assert.equal(value.ok, true, JSON.stringify(value))
    assert.ok(/not permitted|permission denied|denied|os error/i.test(JSON.stringify(value)), JSON.stringify(value))
    assert.ok(!JSON.stringify(value).includes('HOST_ONLY_CANARY'), 'native file tool leaked the host canary')
  }
  assert.equal(fs.readFileSync(canary, 'utf8'), 'HOST_ONLY_CANARY')
  assert.equal(beta.child.exitCode, null)
  assert.equal(beta.child.signalCode, null)
  assert.ok(!seen.some(body => body.tools?.some(t => t.function?.name === 'repl')), 'nested REPL is disabled')
  const handed = await run('LOCAL_HANDOFF')
  assert.equal(handed.ok, true, JSON.stringify(handed))
  const first = seen.find(body => JSON.stringify(body.messages).includes('LOCAL_HANDOFF'))
  assert.ok(JSON.stringify(first.messages).includes('@beta@computer-beta: Beta Research'), 'the next turn sees Beta in its live reminder')
  assert.ok(!JSON.stringify(first.messages).includes('- @alpha'), 'the reminder excludes Alpha itself')
  const outbox = path.join(alpha.root, 'bot_relay/outbox')
  const envelope = JSON.parse(fs.readFileSync(path.join(outbox, fs.readdirSync(outbox).find(p => p.endsWith('.json'))!), 'utf8'))
  assert.equal(envelope.target_profile, 'beta')
  let delivery = await beta.rpc('bot.chat.deliver', { profile: 'beta', envelope_id: envelope.id, text: envelope.message })
  await until(async () => { if (delivery.status === 'pending') delivery = await beta.rpc('bot.chat.deliver_status', { envelope_id: envelope.id }); return delivery.status !== 'pending' })
  assert.equal(delivery.ok, true, JSON.stringify(delivery))
  await alpha.rpc('bot.chat.reply', { id: envelope.id, reply: delivery.reply })
  assert.ok(JSON.stringify(await alpha.rpc('session.history', { session_id: handed.session_id })).includes('BETA_SANDBOX_REPLY'))
  const cron = await alpha.api('/api/cron/jobs?feature=1')
  assert.ok(cron, 'sandboxed Python feature backend answers')
  const job = await alpha.api('/api/cron/jobs?feature=1', { name: '[bot:alpha] Local schedule', schedule: '0 9 * * 1-5', prompt: 'SCHEDULE_LOCAL', deliver: 'bot-chat' })
  assert.ok(job.id, JSON.stringify(job))
  const route = `/api/cron/jobs/${job.id}`
  await alpha.api(`${route}/pause?feature=1`, {})
  assert.equal((await alpha.api(`${route}?feature=1`)).enabled, false)
  assert.notEqual(alpha.launch.env.HOME, beta.launch.env.HOME)
  assert.notEqual(alpha.launch.env.AGENT_BROWSER_PROFILE, beta.launch.env.AGENT_BROWSER_PROFILE)
  await alpha.stop()
  const restarted = await start('alpha')
  assert.equal(fs.readFileSync(path.join(restarted.root, 'workspace/own.txt'), 'utf8'), 'OWN_FILE\n')
  assert.equal(restarted.launch.env.FACTR_HOME, alpha.launch.env.FACTR_HOME)
  assert.equal((await restarted.rpc('bot.chat.ensure', { profile: 'alpha' })).canonical_session.id, canonical)
  assert.equal((await restarted.api(`${route}?feature=1`)).enabled, false, 'paused local schedule survives restart')
  await restarted.api(`${route}/resume?feature=1`, {})
  await restarted.api(`${route}/trigger?feature=1`, {})
  await until(async () => (await restarted.api(`${route}?feature=1`)).last_status === 'ok', 60000)
  assert.ok(JSON.stringify(await restarted.rpc('session.history', { session_id: canonical })).includes('LOCAL_SCHEDULE_RESULT'))
  assert.ok(fs.readdirSync(path.join(restarted.root, 'cron/output'), { recursive: true }).some(file => String(file).endsWith('.md')))
  await restarted.api(`${route}?feature=1`, undefined, 'DELETE')
  await restarted.stop(); await beta.stop()
  console.log('ok: two sandboxed engines, own shell/file tools, denied host read/write/signals, native handoff/reply wake, real scheduled turn/output, pause/restart/resume and canonical persistence')
  console.log(`artifact: ${base}`)
} finally {
  fs.writeFileSync(path.join(base, 'seen.json'), JSON.stringify(seen, null, 2))
  console.log('sandbox artifacts: ' + base)
  for (const child of children) child.kill('SIGKILL')
  model.closeAllConnections(); model.close()
}
