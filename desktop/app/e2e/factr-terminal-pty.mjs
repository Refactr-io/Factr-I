import { _electron } from '@playwright/test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { assertPackagedFactrExists, factrSandboxEnv } from './factr-packaged-paths.mjs'

const executablePath = assertPackagedFactrExists()
const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'factr-terminal-e2e-'))
const marker = 'E2E_TERMINAL_PTY_OK'
let app
try {
  app = await _electron.launch({
    executablePath,
    args: [`--user-data-dir=${path.join(sandbox, 'user-data')}`],
    env: factrSandboxEnv(sandbox),
  })
  const page = await app.firstWindow()
  await page.waitForLoadState('domcontentloaded')
  const result = await page.evaluate(async ({ cwd, marker }) => {
    const terminal = window.factrDesktop.terminal
    const session = await terminal.start({ cwd, cols: 80, rows: 24 })
    if (!session?.id) throw new Error(`PTY start failed: ${JSON.stringify(session)}`)
    await terminal.attach(session.id)
    try {
      const output = await new Promise((resolve, reject) => {
        let text = ''
        let unsubscribe = () => {}
        const timeout = setTimeout(() => {
          unsubscribe()
          reject(new Error(`PTY output timed out: ${text.slice(-1000)}`))
        }, 15_000)
        unsubscribe = terminal.onData(session.id, chunk => {
          text += String(chunk)
          if (text.includes(marker)) {
            clearTimeout(timeout)
            unsubscribe()
            resolve(text)
          }
        })
        void terminal.write(session.id, `printf '%s\\n' '${marker}'\n`)
      })
      return { id: session.id, shell: session.shell, output }
    } finally {
      await terminal.dispose(session.id)
    }
  }, { cwd: sandbox, marker })
  if (!result.output.includes(marker)) throw new Error(`PTY output missed marker: ${JSON.stringify(result)}`)
  console.log(JSON.stringify({ ok: true, shell: result.shell, marker }))
} finally {
  if (app) await app.close().catch(() => {})
  fs.rmSync(sandbox, { recursive: true, force: true })
}
