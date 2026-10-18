// Real Chromium under the production Seatbelt policy. Uses only generated profiles and a local HTTP fixture.
// BOTS_TEST_CHROMIUM=/path/to/Google Chrome for Testing.app desktop/node_modules/.bin/tsx <this file>
import assert from 'node:assert/strict'
import { spawn, spawnSync, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { prepareBotSandbox } from '../../../../desktop/app/electron/bot-local-sandbox.ts'

assert.equal(process.platform, 'darwin', 'this real browser fixture tests macOS Seatbelt')
const app = process.env.BOTS_TEST_CHROMIUM
assert.ok(app && fs.existsSync(app), 'set BOTS_TEST_CHROMIUM to a Chromium app bundle')
const command = path.join(app, 'Contents/MacOS/Google Chrome for Testing')
const base = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'bots-browser-qa-')))
const canary = path.join(base, 'host-canary')
fs.writeFileSync(canary, 'HOST_CANARY_DO_NOT_EXPOSE')
const children = new Set<ChildProcess>()
const escaped = path.join(base, 'escaped')
const probeApp = path.join(base, 'Factr-I sandbox probe.app')
fs.mkdirSync(path.join(probeApp, 'Contents/MacOS'), { recursive: true })
fs.writeFileSync(path.join(probeApp, 'Contents/Info.plist'), '<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>probe</string><key>CFBundleIdentifier</key><string>net.refactr.sandbox-probe</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>')
fs.writeFileSync(path.join(probeApp, 'Contents/MacOS/probe'), `#!/bin/sh\necho ESCAPED > '${escaped}'\n`, { mode: 0o755 })
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
async function until(test: () => boolean | Promise<boolean>, timeout = 20000) {
  const deadline = Date.now() + timeout
  while (!await test()) { assert.ok(Date.now() < deadline, 'browser condition timed out'); await sleep(50) }
}
const fixture = http.createServer((req, res) => {
  if (req.url === '/set') res.setHeader('set-cookie', 'bot=alpha; Max-Age=3600; SameSite=Lax')
  res.setHeader('content-type', 'text/html')
  res.end('<h1>Factr-I private browser test</h1>')
})
await new Promise<void>(resolve => fixture.listen(0, '127.0.0.1', resolve))
const url = `http://127.0.0.1:${(fixture.address() as any).port}`

async function browser(profile: string) {
  const root = path.join(base, 'profiles', profile)
  fs.mkdirSync(root, { recursive: true })
  const launch = prepareBotSandbox({ profile, configHome: base, command,
    args: ['--headless=new', '--no-sandbox', '--disable-setuid-sandbox', '--disable-gpu', '--no-first-run',
      '--disable-crash-reporter', '--use-mock-keychain', '--password-store=basic', '--remote-debugging-port=0', `--user-data-dir=${root}/browser-profile`, 'about:blank'],
    env: process.env, readRoots: [app!] })
  const child = spawn(launch.command, launch.args, { cwd: launch.cwd, env: launch.env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
  children.add(child)
  let output = ''
  child.stderr.on('data', data => { output += data })
  child.stdout.on('data', data => { output += data })
  await until(() => /DevTools listening on ws:/.test(output) || child.exitCode !== null || child.signalCode !== null)
  assert.equal(child.exitCode, null, output)
  assert.equal(child.signalCode, null, output)
  const endpoint = output.match(/DevTools listening on (ws:\/\/[^\s]+)/)![1]
  const ws = new WebSocket(endpoint)
  let next = 0
  const pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> }>()
  ws.onmessage = event => {
    const frame = JSON.parse(String(event.data)), entry = pending.get(frame.id)
    if (entry) { pending.delete(frame.id); clearTimeout(entry.timer); frame.error ? entry.reject(new Error(JSON.stringify(frame.error))) : entry.resolve(frame.result) }
  }
  await new Promise<void>((resolve, reject) => { ws.onopen = () => resolve(); ws.onerror = reject })
  const call = (method: string, params: any = {}, sessionId?: string) => new Promise<any>((resolve, reject) => {
    const id = ++next, timer = setTimeout(() => { pending.delete(id); reject(new Error(method + ' timed out')) }, 10000)
    pending.set(id, { resolve, reject, timer })
    ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }))
  })
  const target = (await call('Target.getTargets')).targetInfos.find((t: any) => t.type === 'page')
  const session = (await call('Target.attachToTarget', { targetId: target.targetId, flatten: true })).sessionId
  const evaluate = async (expression: string) => (await call('Runtime.evaluate', { expression, returnByValue: true }, session)).result.value
  const navigate = async (href: string) => {
    const result = await call('Page.navigate', { url: href }, session)
    if (!result.errorText) await until(async () => await evaluate('document.readyState') === 'complete' && await evaluate('location.href') === href)
    return result
  }
  return { launch, root, evaluate, navigate, close: async () => {
    await call('Browser.close')
    await until(() => child.exitCode !== null || child.signalCode !== null, 10000)
    ws.close(); children.delete(child); launch.cleanup()
  } }
}

try {
  const control = spawnSync('/usr/bin/open', ['-W', '-n', probeApp], { encoding: 'utf8', timeout: 10000 })
  assert.equal(control.status, 0, control.stderr)
  assert.ok(fs.existsSync(escaped), 'the unsandboxed disposable app must prove the probe works')
  fs.unlinkSync(escaped)
  const alpha = await browser('alpha')
  await alpha.navigate(url + '/set')
  assert.equal(await alpha.evaluate('document.cookie'), 'bot=alpha')
  const beta = await browser('beta')
  await beta.navigate(url + '/get')
  assert.equal(await beta.evaluate('document.cookie'), '', 'a second bot has a different cookie jar')
  await beta.navigate('file://' + canary)
  assert.ok(!String(await beta.evaluate('document.body.innerText')).includes('HOST_CANARY_DO_NOT_EXPOSE'))
  // A browser's startup services must not let a shell escape through LaunchServices.
  const probe = spawnSync('/usr/bin/sandbox-exec', ['-p', beta.launch.args[1], '/usr/bin/open', '-W', '-n', probeApp],
    { cwd: beta.launch.cwd, env: beta.launch.env, encoding: 'utf8', timeout: 10000 })
  assert.notEqual(probe.status, 0, 'external app launch must be denied: ' + probe.stderr)
  assert.ok(!fs.existsSync(escaped), 'a broker must not execute outside the sandbox')
  await beta.close(); await alpha.close()
  const reopened = await browser('alpha')
  await reopened.navigate(url + '/get')
  assert.equal(await reopened.evaluate('document.cookie'), 'bot=alpha', 'the bot keeps its own login storage across restart')
  await reopened.close()
  console.log('ok: real browser navigation, private cookie jars, denied host file/LaunchServices access and persistent storage')
  console.log('artifact: ' + base)
} finally {
  for (const child of children) { try { process.kill(-child.pid!, 'SIGKILL') } catch {} }
  fixture.closeAllConnections(); fixture.close()
}
