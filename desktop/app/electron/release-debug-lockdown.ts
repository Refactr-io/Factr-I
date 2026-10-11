/**
 * Release-build lockdown of Chromium's remote debugging switches.
 *
 * Electron fuses switch off RunAsNode, NODE_OPTIONS and --inspect, but not
 * Chromium's own `--remote-debugging-port` / `--remote-debugging-pipe`: anyone
 * who can launch the app binary with that flag gets the DevTools protocol
 * (arbitrary code in the renderer). The release variant therefore drops those
 * switches from the command line at the very top of the main process, before
 * the DevTools server reads them. The QA variant (built with FACTR_QA_FUSES=1)
 * keeps them: the QA harness attaches over that port. An unpackaged run
 * (`electron .`, `npm run dev`, the perf and diag scripts) is a developer
 * build and is left alone.
 *
 * The variant is a build-time constant (esbuild `define` in
 * scripts/bundle-electron-main.mjs, from the same FACTR_QA_FUSES variable that
 * drives the fuse hook), never a runtime environment variable, so setting an
 * env var cannot turn the port back on in a release build.
 */

export const REMOTE_DEBUGGING_SWITCHES = [
  'remote-debugging-port',
  'remote-debugging-pipe',
  'remote-debugging-address',
  'remote-debugging-io-pipes',
  'remote-debugging-targets',
  'remote-allow-origins'
] as const

/** Whether a Chromium switch name (no leading dashes) is one of the remote debugging switches. */
export function isRemoteDebuggingSwitch(name: string): boolean {
  return (REMOTE_DEBUGGING_SWITCHES as readonly string[]).includes(name.trim().replace(/^-+/, ''))
}

/**
 * Whether THIS bundle is the QA variant. `process.env.FACTR_QA_BUILD` is replaced at bundle time by esbuild
 * (scripts/bundle-electron-main.mjs: "1" for FACTR_QA_FUSES=1, "0" otherwise), so the runtime environment can
 * never change it. A bundle without the define (dev) reads as not-QA: that fails closed.
 */
export function isQaBuild(): boolean {
  return process.env.FACTR_QA_BUILD === '1'
}

export interface CommandLineLike {
  hasSwitch(name: string): boolean
  removeSwitch(name: string): void
}

/** True for a packaged build that is not the QA variant. */
export function isLockedDownBuild(isPackaged: boolean, qaBuild: boolean): boolean {
  return isPackaged && !qaBuild
}

/** Remove the remote debugging switches; returns the names that were present. */
export function stripRemoteDebuggingSwitches(commandLine: CommandLineLike): string[] {
  const removed: string[] = []

  for (const name of REMOTE_DEBUGGING_SWITCHES) {
    if (commandLine.hasSwitch(name)) {
      commandLine.removeSwitch(name)
      removed.push(name)
    }
  }

  return removed
}
