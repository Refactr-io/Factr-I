// End-to-end proof that "Update now" really updates the app, on Windows and Linux (CI: .github/workflows/update-e2e.yml).
// Installs the OLD packaged build (0.0.14, QA fuses on) the way a user would, serves the NEW build (0.0.15) from a local fake
// release feed, presses Update now in the real UI, and then checks the new version is the one running: the new app writes
// the health marker <userData>/updates/<version>/ok only when its own version equals the version it was updated to.
//
// usage: node scripts/e2e-update-now.mjs <dir with the old installer> <dir with the new installer> [good|corrupt]
import { _electron as electron } from '@playwright/test'
import { execFileSync, spawnSync } from 'node:child_process'
import crypto from 'node:crypto'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'

const [oldDir, newDir, mode = 'good'] = process.argv.slice(2)
const OLD_V = '0.0.14'
const NEW_V = '0.0.15'
const WIN = process.platform === 'win32'
const ROOT = path.join(process.env.RUNNER_TEMP || os.tmpdir(), `update-e2e-${Date.now()}`)
const UD = path.join(ROOT, 'userData')
const CFG = path.join(ROOT, 'cfg')
const SHOTS = path.join(ROOT, 'shots')
const sleep = ms => new Promise(r => setTimeout(r, ms))
const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a)

for (const dir of [UD, CFG, SHOTS]) {
  fs.mkdirSync(dir, { recursive: true })
}

const names = WIN
  ? { old: `Factr-I-${OLD_V}-win-x64.exe`, next: `Factr-I-${NEW_V}-win-x64.exe` }
  : { old: `Factr-I-${OLD_V}-linux-x86_64.AppImage`, next: `Factr-I-${NEW_V}-linux-x86_64.AppImage` }

const newFile = path.join(newDir, names.next)
const payload = fs.readFileSync(newFile)
const sha = crypto.createHash('sha256').update(payload).digest('hex')
// the "corrupt" mode serves the right size with wrong bytes: the app must refuse it and stay on the old version
const served = mode === 'corrupt' ? Buffer.concat([payload.subarray(0, payload.length - 1000), Buffer.alloc(1000, 7)]) : payload
let port = 0
let downloads = 0

const server = http.createServer(async (req, res) => {
  if (req.url?.startsWith('/releases/latest')) {
    res.setHeader('content-type', 'application/json')
    res.end(
      JSON.stringify({
        tag_name: `v${NEW_V}`,
        html_url: `https://github.com/Refactr-io/Factr-I/releases/tag/v${NEW_V}`,
        body: 'End-to-end test release',
        assets: [{ name: names.next, size: payload.length, digest: `sha256:${sha}`, browser_download_url: `http://127.0.0.1:${port}/dl/${names.next}` }]
      })
    )

    return
  }

  if (req.url?.startsWith('/dl/')) {
    downloads += 1
    res.writeHead(200, { 'content-length': served.length, 'content-type': 'application/octet-stream' })

    for (let i = 0; i < served.length; i += 1 << 20) {
      if (!res.write(served.subarray(i, i + (1 << 20)))) {
        await new Promise(r => res.once('drain', r))
      }
    }

    res.end()

    return
  }

  res.statusCode = 404
  res.end()
})

await new Promise(r => server.listen(0, '127.0.0.1', r))
port = server.address().port
log('feed on', port, '| new build', names.next, payload.length, 'bytes')

// 1. install the old build the way a user has it
let launchPath
let installedFile

if (WIN) {
  const installer = path.join(oldDir, names.old)

  log('installing', installer, 'silently')
  const result = spawnSync(installer, ['/S'], { timeout: 300000 })

  if (result.status !== 0) {
    throw new Error(`the old installer exited with ${result.status}`)
  }

  const installDir = path.join(process.env.LOCALAPPDATA, 'Programs', 'Factr-I')

  launchPath = path.join(installDir, 'Factr-I.exe')

  if (!fs.existsSync(launchPath) || !fs.existsSync(path.join(installDir, 'Uninstall Factr-I.exe'))) {
    throw new Error(`the old build did not install to ${installDir}`)
  }
} else {
  const appsDir = path.join(os.homedir(), 'Applications')

  fs.mkdirSync(appsDir, { recursive: true })
  launchPath = path.join(appsDir, 'Factr-I.AppImage')
  fs.copyFileSync(path.join(oldDir, names.old), launchPath)
  fs.chmodSync(launchPath, 0o755)
  installedFile = launchPath
}

// 2. start it, auto-download off so nothing happens until the user presses Update now
fs.writeFileSync(path.join(UD, 'update-auto-download.json'), JSON.stringify({ enabled: false }))

const env = {
  ...process.env,
  FACTR_CONFIG_HOME: CFG,
  FACTR_HOME: path.join(CFG, 'engine'),
  FACTR_DESKTOP_USER_DATA_DIR: UD,
  FACTR_UPDATE_FEED_URL: `http://127.0.0.1:${port}/releases/latest`,
  FACTR_SWAP_HEALTH_WAIT: '150',
  OLLAMA_HOST: '127.0.0.1:9'
}

const app = await electron.launch({ executablePath: launchPath, args: WIN ? [] : ['--no-sandbox'], env, timeout: 120000 })
const page = await app.firstWindow()

await page.waitForLoadState('domcontentloaded')
log('old app up, version', await app.evaluate(({ app: a }) => a.getVersion()))
await sleep(9000)

// a fresh home starts on the model picker; skip it like a user who chooses later
const later = page.getByText(/choose a provider later/i).first()

if (await later.count()) {
  await later.click({ timeout: 5000 }).catch(() => undefined)
  await sleep(4000)
}

await page.evaluate(() => window.factrDesktop.updates.check({ force: true }))
await sleep(2500)
await page.screenshot({ path: path.join(SHOTS, '1-before.png') })

// 3. press Update now (status-bar update item -> dialog -> Update now)
await page.getByRole('button', { name: /^update · Factr-I/i }).first().click({ timeout: 15000 })
await sleep(1500)
await page.screenshot({ path: path.join(SHOTS, '2-dialog.png') })
await page.getByRole('button', { name: /^update now$/i }).first().click({ timeout: 15000 })
log('pressed Update now')

// 4. watch until the app quits for the swap (or the update is refused)
const t0 = Date.now()
let sawMascot = false
let last = ''

while (Date.now() - t0 < 360000) {
  const splash = app.windows().find(w => w.url().startsWith('data:text/html'))

  if (splash && !sawMascot) {
    sawMascot = true
    await splash.screenshot({ path: path.join(SHOTS, '3-mascot.png') }).catch(() => undefined)
    log('mascot update screen is on screen')
  }

  const text = await page.evaluate(() => document.body.innerText).catch(() => null)

  if (text === null) {
    log('the window is gone: the app quit for the swap')
    break
  }

  const line = (text.match(/(Downloading[^\n]*|\d+ of \d+ MB|Update didn.t finish[^\n]*)/g) ?? []).join(' | ')

  if (line !== last) {
    last = line
    log('ui:', line || '(dialog closed)')
  }

  if (/Update didn.t finish/.test(text)) {
    await page.screenshot({ path: path.join(SHOTS, '4-refused.png') }).catch(() => undefined)
    break
  }

  await sleep(1000)
}

// 5. wait for the helper to finish, then check what is installed
const updates = path.join(UD, 'updates')
const helperLog = path.join(updates, 'helper.log')
const marker = path.join(updates, NEW_V, 'ok')

if (mode === 'good') {
  for (let i = 0; i < 300; i += 1) {
    const h = fs.existsSync(helperLog) ? fs.readFileSync(helperLog, 'utf8') : ''

    if (fs.existsSync(marker) || /rollback done|abort|installer did not finish/.test(h)) {
      break
    }

    await sleep(1000)
  }
}

await sleep(3000)

const helper = fs.existsSync(helperLog) ? fs.readFileSync(helperLog, 'utf8') : '(no helper log)'

console.log('--- helper.log\n' + helper)
console.log('--- downloads served:', downloads, '| mascot seen:', sawMascot)

let pass
let detail

if (mode === 'good') {
  const healthy = fs.existsSync(marker)
  const sameFile = installedFile ? crypto.createHash('sha256').update(fs.readFileSync(installedFile)).digest('hex') === sha : true

  pass = healthy && sameFile && /ok: the new app reported healthy/.test(helper) && sawMascot
  detail = `new app reported healthy: ${healthy}; installed file is the new build: ${sameFile}; mascot shown: ${sawMascot}`
} else {
  const refused = fs.existsSync(path.join(SHOTS, '4-refused.png'))
  const untouched = installedFile ? fs.readFileSync(installedFile).length !== payload.length : true

  pass = refused && untouched && !fs.existsSync(marker)
  detail = `refusal shown: ${refused}; old build untouched: ${untouched}`
}

console.log(pass ? `RESULT: PASS (${mode}) ${detail}` : `RESULT: FAIL (${mode}) ${detail}`)

try {
  if (WIN) {
    execFileSync('taskkill', ['/IM', 'Factr-I.exe', '/F', '/T'], { stdio: 'ignore' })
  } else {
    execFileSync('pkill', ['-f', 'Factr-I.AppImage'], { stdio: 'ignore' })
  }
} catch {
  // nothing left running
}

server.close()
process.exit(pass ? 0 : 1)
