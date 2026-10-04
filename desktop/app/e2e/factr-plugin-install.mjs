import { _electron } from '@playwright/test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { assertPackagedFactrExists, factrSandboxEnv } from './factr-packaged-paths.mjs'

const executablePath = assertPackagedFactrExists()
const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'factr-plugin-e2e-'))
const repository = path.join(sandbox, 'plugin-repository')
const marker = 'E2E_PLUGIN_CAPABILITY_ENABLED'
fs.mkdirSync(repository, { recursive: true })
fs.writeFileSync(path.join(repository, 'plugin.js'), `export default {
  id: 'e2e-installed-plugin',
  name: 'E2E Installed Plugin',
  description: 'Local packaged install walkthrough fixture',
  defaultEnabled: true,
  register(context) {
    context.register({
      id: 'capability-marker',
      area: 'statusBar.right',
      data: { id: 'e2e-plugin-capability', label: '${marker}', variant: 'text' }
    })
  }
}\n`)
execFileSync('git', ['init', '--quiet', repository])
execFileSync('git', ['-C', repository, 'config', 'user.name', 'Factr E2E'])
execFileSync('git', ['-C', repository, 'config', 'user.email', 'factr-e2e@example.invalid'])
execFileSync('git', ['-C', repository, 'add', 'plugin.js'])
execFileSync('git', ['-C', repository, 'commit', '--quiet', '-m', 'plugin fixture'])

let app
try {
  app = await _electron.launch({
    executablePath,
    args: [`--user-data-dir=${path.join(sandbox, 'user-data')}`],
    env: factrSandboxEnv(sandbox),
  })
  const page = await app.firstWindow()
  await page.waitForLoadState('domcontentloaded')
  const installed = await page.evaluate(async identifier =>
    window.factrDesktop.installDesktopPlugin({ identifier }),
  `file://${repository}`)
  if (!installed?.ok) throw new Error(`Local plugin installation failed: ${JSON.stringify(installed)}`)

  const pluginPath = path.join(installed.path, 'plugin.js')
  await page.waitForFunction(async pathname => {
    const result = await window.factrDesktop.readFileText(pathname)
    return result.ok && result.text.includes('e2e-installed-plugin')
  }, pluginPath, { timeout: 15_000 })
  await page.getByText(marker, { exact: true }).waitFor({ timeout: 30_000 })
  console.log(JSON.stringify({ ok: true, pluginName: installed.pluginName, capability: marker }))
} finally {
  if (app) await app.close()
  fs.rmSync(sandbox, { recursive: true, force: true })
}
