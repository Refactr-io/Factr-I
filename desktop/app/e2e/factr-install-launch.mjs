import { _electron } from '@playwright/test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  assertPackagedFactrExists,
  factrSandboxEnv,
} from './factr-packaged-paths.mjs'
import { isPythonComm, isFactrComm, processTree } from './factr-process-utils.mjs'

const desktop = path.resolve(import.meta.dirname, '..')
const executablePath = assertPackagedFactrExists()
const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'factr-install-'))
const output = process.env.FACTR_INSTALL_E2E_OUTPUT || path.join(desktop, 'release/factr-install')
fs.mkdirSync(output, { recursive: true })

const app = await _electron.launch({
  executablePath,
  args: [`--user-data-dir=${path.join(sandbox, 'user-data')}`],
  env: factrSandboxEnv(sandbox),
})
try {
  const page = await app.firstWindow()
  const mainPid = app.process().pid
  await page.waitForLoadState('domcontentloaded')
  await page.waitForTimeout(10_000)
  await page.screenshot({ path: path.join(output, 'first-run.png'), animations: 'disabled' })
  const text = (await page.locator('body').innerText()).slice(0, 3000)
  const processes = processTree(mainPid)
  fs.writeFileSync(path.join(output, 'first-run.txt'), `${text}\n\n${JSON.stringify(processes, null, 2)}\n`)
  if (processes.filter(row => isFactrComm(row.comm)).length !== 1) {
    throw new Error(`Expected one Factr process; tree=${JSON.stringify(processes)}`)
  }
  if (processes.some(row => isPythonComm(row.comm))) {
    throw new Error(`Python started before a feature opened; tree=${JSON.stringify(processes)}`)
  }

  await page.getByText('Scheduled jobs', { exact: true }).click()
  await page.getByText('0 jobs').waitFor({ timeout: 120_000 })
  // Python answered the page just now; sample before the (short) idle stop.
  const cronProcesses = processTree(mainPid)
  await page.screenshot({ path: path.join(output, 'cron.png'), animations: 'disabled' })
  fs.writeFileSync(path.join(output, 'cron-processes.json'), JSON.stringify(cronProcesses, null, 2))
  if (!cronProcesses.some(row => isPythonComm(row.comm))) {
    throw new Error(`Cron did not start bundled Python; tree=${JSON.stringify(cronProcesses)}`)
  }

  await page.keyboard.press('Escape')
  await page.waitForTimeout(300)
  await page.keyboard.press('Escape')
  await page.getByText('0 jobs').waitFor({ state: 'hidden', timeout: 15_000 })
  let idleProcesses = processTree(mainPid)
  for (const until = Date.now() + 20_000; idleProcesses.some(row => isPythonComm(row.comm)) && Date.now() < until; ) {
    await page.waitForTimeout(500)
    idleProcesses = processTree(mainPid)
  }
  if (idleProcesses.some(row => isPythonComm(row.comm))) {
    throw new Error('Python did not stop after idle timeout')
  }
  fs.writeFileSync(path.join(output, 'idle.json'), JSON.stringify({ sandbox, processes, cronProcesses, idleProcesses }, null, 2))
  console.log(JSON.stringify({ sandbox, processes, cronProcesses, idleProcesses }, null, 2))
} finally {
  await app.close()
}
