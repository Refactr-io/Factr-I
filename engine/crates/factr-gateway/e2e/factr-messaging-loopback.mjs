#!/usr/bin/env node
// Configure Factr's API-server messaging platform through its dashboard API,
// then deliver one OpenAI-compatible chat message over loopback only.
import { spawn } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'

const engineRoot = path.resolve(import.meta.dirname, '../../..')
const factrRoot = process.env.FACTR_REPO || path.resolve(engineRoot, '../backend')
const python = process.env.FACTR_PYTHON || path.join(factrRoot, '.venv/bin/python')
const model = process.env.E2E_MODEL || 'factr/bench-factr-64k:latest'
const home = fs.mkdtempSync(path.join(os.tmpdir(), 'factr-messaging-'))
const factrHome = path.join(home, '.factr')
const factrHome = path.join(home, '.factr/engine')
const runtimeDir = path.join(os.tmpdir(), `sov-msg-${process.pid}-${crypto.randomBytes(3).toString('hex')}`)
const dashboardToken = crypto.randomBytes(24).toString('hex')
const apiKey = crypto.randomBytes(32).toString('hex')
const replyMarker = `messaging-loopback-${crypto.randomBytes(4).toString('hex')}`
const children = []
fs.mkdirSync(factrHome, { recursive: true })
fs.mkdirSync(factrHome, { recursive: true })
fs.mkdirSync(runtimeDir, { recursive: true })
fs.writeFileSync(path.join(factrHome, 'config.yaml'), [
  'model:',
  `  default: ${model}`,
  '  provider: ollama',
  '  base_url: http://127.0.0.1:11434/v1',
  '  api_mode: chat_completions',
  '',
].join('\n'))

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const check = (condition, message) => { if (!condition) throw new Error(message) }
const start = (args) => {
  const child = spawn(python, args, {
    cwd: factrRoot,
    env: {
      ...process.env,
      HOME: home,
      FACTR_CONFIG_HOME: factrHome,
      FACTR_HOME: factrHome,
      FACTR_RUNTIME_DIR: runtimeDir,
      FACTR_DASHBOARD_SESSION_TOKEN: dashboardToken,
      FACTR_ACCEPT_HOOKS: '1',
      FACTR_GATEWAY_NO_SUPERVISE: '1',
      PYTHONUNBUFFERED: '1',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdoutText = ''
  child.stderrText = ''
  child.stdout.on('data', data => { child.stdoutText += data })
  child.stderr.on('data', data => { child.stderrText += data })
  children.push(child)
  return child
}
const waitFor = async (description, probe, child, timeoutMs = 30_000) => {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (child?.exitCode !== null && child?.exitCode !== undefined) {
      throw new Error(`${description}: process exited ${child.exitCode}\n${child.stderrText}`)
    }
    try {
      const result = await probe()
      if (result) return result
    } catch {}
    await sleep(150)
  }
  throw new Error(`${description}: timed out\n${child?.stderrText || ''}`)
}
const freePort = async () => new Promise((resolve, reject) => {
  const server = net.createServer()
  server.once('error', reject)
  server.listen(0, '127.0.0.1', () => {
    const { port } = server.address()
    server.close(error => error ? reject(error) : resolve(port))
  })
})

let gateway
try {
  const dashboard = start(['-m', 'factr_backend.main', 'serve', '--host', '127.0.0.1', '--port', '0', '--skip-build'])
  const dashboardPort = await waitFor('Factr dashboard', async () => {
    const match = dashboard.stdoutText.match(/FACTR_BACKEND_READY port=(\d+)/)
    return match && Number(match[1])
  }, dashboard)
  const dashboardUrl = `http://127.0.0.1:${dashboardPort}`
  const dashboardHeaders = { 'content-type': 'application/json', 'X-Factr-Session-Token': dashboardToken }
  const update = await fetch(`${dashboardUrl}/api/messaging/platforms/api_server`, {
    method: 'PUT',
    headers: dashboardHeaders,
    body: JSON.stringify({ enabled: true, env: {
      API_SERVER_KEY: apiKey,
      API_SERVER_HOST: '127.0.0.1',
      API_SERVER_PORT: String(await freePort()),
    } }),
  })
  check(update.ok, `Factr messaging config update failed ${update.status}: ${await update.text()}`)
  const listingResponse = await fetch(`${dashboardUrl}/api/messaging/platforms`, { headers: dashboardHeaders })
  check(listingResponse.ok, `Factr messaging config read failed ${listingResponse.status}`)
  const listing = await listingResponse.json()
  const apiServer = listing.platforms?.find(platform => platform.id === 'api_server')
  check(apiServer?.enabled && apiServer.configured, `API server config was not persisted: ${JSON.stringify(apiServer)}`)
  const keyField = apiServer.env_vars?.find(field => field.key === 'API_SERVER_KEY')
  check(keyField?.is_set && keyField.redacted_value && !JSON.stringify(apiServer).includes(apiKey), 'API key readback was missing or not redacted')

  gateway = start(['-m', 'factr_backend.main', 'gateway', 'run', '--no-supervise', '--accept-hooks'])
  const port = Number((fs.readFileSync(path.join(factrHome, '.env'), 'utf8').match(/^API_SERVER_PORT=(\d+)$/m) || [])[1])
  check(Number.isInteger(port) && port > 0, 'configured loopback API server port was not written')
  const base = `http://127.0.0.1:${port}`
  await waitFor('API server health', async () => {
    const response = await fetch(`${base}/health`)
    return response.ok
  }, gateway, 60_000)
  const response = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST',
    headers: { authorization: `Bearer ${apiKey}`, 'content-type': 'application/json' },
    body: JSON.stringify({
      model,
      messages: [{ role: 'user', content: `Reply with exactly this text and nothing else: ${replyMarker}` }],
      stream: false,
    }),
  })
  const body = await response.json()
  check(response.ok, `loopback API-server message failed ${response.status}: ${JSON.stringify(body)}`)
  check(body.choices?.[0]?.message?.content?.trim() === replyMarker, `loopback message reply mismatch: ${JSON.stringify(body.choices?.[0]?.message)}`)
  console.log('PASS Factr messaging API saved and read back isolated api_server configuration with key redaction')
  console.log('PASS loopback API-server message reached local Ollama and returned its response')
} finally {
  for (const child of [...children].reverse()) {
    if (child.exitCode === null) child.kill('SIGTERM')
  }
  await Promise.all(children.map(child => new Promise(resolve => {
    if (child.exitCode !== null) return resolve()
    child.once('exit', resolve)
    setTimeout(() => { child.kill('SIGKILL'); resolve() }, 5000)
  })))
  for (const child of children) { child.stdout.destroy(); child.stderr.destroy() }
  fs.rmSync(runtimeDir, { recursive: true, force: true })
  if (!process.env.KEEP_MESSAGING_HOME) fs.rmSync(home, { recursive: true, force: true })
  else console.log(`messaging home: ${home}`)
}
