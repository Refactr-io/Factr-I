// One persistent Desktop socket must survive idle retirement of its Python feature backend.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'bots-feature-link-'))
const config = path.join(root, 'config'), home = path.join(root, 'engine')
fs.mkdirSync(config); fs.mkdirSync(home)
fs.writeFileSync(path.join(config, 'config.yaml'), 'model:\n  provider: custom\n  default: unused-stub\n  base_url: http://127.0.0.1:1/v1\nplugins:\n  enabled: []\n')
const token = 'synthetic-feature-reconnect-test-token'
const child = spawn(process.env.FACTR_BIN, ['serve', '--host', '127.0.0.1', '--port', '0', '--token-stdin'], {
  cwd: root, env: { ...process.env, HOME: root, FACTR_CONFIG_HOME: config, FACTR_HOME: home, FACTR_RUNTIME_DIR: path.join(root, 'runtime'),
    FACTR_FEATURE_IDLE_MS: '5000', FACTR_TRACE_FEATURE_ACTIVITY: '1', OLLAMA_HOST: '127.0.0.1:1' }, stdio: ['pipe', 'pipe', 'pipe']
})
let output = '', ws
child.stdout.on('data', data => { output += data }); child.stderr.on('data', data => { output += data })
child.stdin.end(token + '\n')
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
async function until(test, ms = 30000) {
  const end = Date.now() + ms
  while (!test()) { assert.ok(Date.now() < end, output); await sleep(50) }
}
try {
  await until(() => /FACTR_BACKEND_READY port=\d+/.test(output) || child.exitCode !== null)
  assert.equal(child.exitCode, null, output)
  const port = output.match(/FACTR_BACKEND_READY port=(\d+)/)[1]
  ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`)
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject })
  let next = 0
  const pending = new Map()
  ws.onmessage = event => { const frame = JSON.parse(String(event.data)); pending.get(frame.id)?.(frame) }
  async function rpc(method, params = {}) {
    const id = ++next
    const reply = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method}: feature socket failed to reconnect\n${output}`)) }, 60000)
      pending.set(id, value => { pending.delete(id); clearTimeout(timer); resolve(value) })
      ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
    })
    assert.ok(!reply.error, JSON.stringify(reply.error))
    return reply.result
  }
  await rpc('cron.manage', { action: 'list' })
  for (let round = 0; round < 2; round++) {
    const before = output.length
    await until(() => output.slice(before).includes('feature_idle decision=stop'))
    await sleep(1000) // Allow the feature WebSocket EOF to reach the Rust reader.
    assert.equal(ws.readyState, WebSocket.OPEN, 'Desktop transport itself must remain open')
    await rpc('ping')
    await rpc('cron.manage', { action: 'list' })
  }
  console.log('PASS persistent Desktop socket reconnects feature RPC after two real idle backend exits')
} finally {
  ws?.close(); child.kill('SIGTERM')
  await until(() => child.exitCode !== null || child.signalCode !== null, 10000).catch(() => child.kill('SIGKILL'))
  fs.writeFileSync(path.join(root, 'engine-output.log'), output)
  console.log('artifact: ' + root)
}
