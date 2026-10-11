import { app } from 'electron'

import { isLockedDownBuild, isQaBuild, stripRemoteDebuggingSwitches } from './release-debug-lockdown'
import { wslgLaunchArgs } from './wslg-launch'
import { spawnWslgLaunch } from './wslg-launch-process'

// Release build: drop Chromium's remote debugging switches before anything else runs (the DevTools server reads
// the command line only after this script has run). Runs on every launch, relaunches included, and never exits:
// a stray flag must not turn into a start failure or a relaunch loop. See release-debug-lockdown.ts.
if (isLockedDownBuild(app.isPackaged, isQaBuild())) {
  const removed = stripRemoteDebuggingSwitches(app.commandLine)

  if (removed.length > 0) {
    console.warn(`[factr] ignored remote debugging switches: ${removed.map(name => `--${name}`).join(' ')}`)
  }
}

const args = wslgLaunchArgs(process.argv.slice(1), process.env, process.platform)

if (args) {
  // Keep the launcher alive until the child exits: npm's concurrently must not
  // tear down Vite during this handoff. No backend, windows or single-instance
  // lock are created in this parent. The child has an explicit platform flag,
  // so it goes straight into main on its first pass.
  const child = spawnWslgLaunch(args)

  child.once('error', error => {
    console.error('[factr] WSLg launch failed:', error)
    app.exit(1)
  })
  child.once('exit', code => app.exit(code ?? 1))

  for (const signal of ['SIGINT', 'SIGTERM'] as const) {
    process.once(signal, () => child.kill(signal))
  }
} else {
  await import('./main')
}
