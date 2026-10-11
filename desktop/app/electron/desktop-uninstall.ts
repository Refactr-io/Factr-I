/**
 * desktop-uninstall.ts
 *
 * Pure, electron-free helpers for the desktop Chat GUI uninstaller. These map
 * the three user-facing uninstall modes to the `factr uninstall` CLI flags,
 * resolve the running app bundle/exe so a detached cleanup script can remove
 * it after the app quits, and build that cleanup script for each OS.
 *
 * Kept standalone (no ` import 'electron'`) so it can be unit-tested with
 * `node --test` — same pattern as connection-config.ts / backend-probes.ts.
 * main.ts requires these and wires them into the electron-coupled IPC layer.
 *
 * The three modes mirror the CLI's options exactly:
 *   - 'gui'  → remove ONLY the Chat GUI, keep the agent + all user data.
 *              `factr uninstall --gui --yes`
 *   - 'lite' → remove the GUI + agent code, KEEP user data (config / sessions
 *              / .env) for a future reinstall. `factr uninstall --yes`
 *   - 'full' → remove everything: GUI + agent + all user data.
 *              `factr uninstall --full --yes`
 *
 * Why a detached cleanup script: 'lite'/'full' delete the very venv the
 * `factr` command runs from, and every mode may need to delete the running
 * app bundle (locked on macOS/Windows while the process is alive). So we hand
 * the work to a detached child that waits for this app's PID to exit, runs the
 * Python uninstall, then removes the app bundle — then the app quits. Same
 * shape as the self-update swap-and-relaunch flow already in main.ts.
 */

import path from 'node:path'

const UNINSTALL_MODES = ['gui', 'lite', 'full']

/**
 * Map an uninstall mode to the `python -m factr_backend.uninstall` argv (after the
 * python executable). Uses the dedicated lightweight module entrypoint (not
 * `factr_backend.main`) so it can run under a system Python OUTSIDE the venv that
 * lite/full delete — see the Finding-3 note in buildWindowsCleanupScript.
 * Throws on an unknown mode so a typo can't silently become a full wipe.
 */
function uninstallArgsForMode(mode) {
  if (!UNINSTALL_MODES.includes(mode)) {
    throw new Error(`Unknown uninstall mode: ${mode}`)
  }

  return ['-m', 'factr_backend.uninstall', '--mode', mode]
}

/** True when `mode` removes the agent (lite/full), false for gui-only. */
function modeRemovesAgent(mode) {
  return mode === 'lite' || mode === 'full'
}

/** True when `mode` removes user data (full only). */
function modeRemovesUserData(mode) {
  return mode === 'full'
}

/**
 * Resolve the on-disk app bundle/dir to remove for the running desktop app,
 * given the path to the running executable (`process.execPath`) and platform.
 *
 *   macOS:   …/Factr.app/Contents/MacOS/Factr  → …/Factr.app
 *   Windows: …\Factr\Factr.exe                 → …\Factr  (install dir)
 *   Linux:   AppImage → the APPIMAGE env path; unpacked → the *-unpacked dir
 *
 * Returns null when we can't confidently identify a removable bundle (e.g.
 * running from a dev checkout, or a system-package install we must not rmtree).
 */
function resolveRemovableAppPath(execPath, platform, env: any = {}) {
  const exe = String(execPath || '')

  if (!exe) {
    return null
  }

  // Use the path flavor that matches the TARGET platform, not the host running
  // this code — so the Windows branch parses backslash paths correctly even
  // when these pure helpers are unit-tested on Linux/macOS CI.
  const p = platform === 'win32' ? path.win32 : path.posix

  if (platform === 'darwin') {
    // …/Factr-I.app/Contents/MacOS/Factr-I → strip 3 segments to the .app
    const macOsDir = p.dirname(exe) // …/Contents/MacOS
    const contents = p.dirname(macOsDir) // …/Contents
    const appBundle = p.dirname(contents) // …/Factr-I.app

    if (appBundle.endsWith('.app')) {
      return appBundle
    }

    return null
  }

  if (platform === 'win32') {
    // NSIS per-user installs Factr-I.exe directly in the install dir.
    const dir = p.dirname(exe)

    if (/[\\/]Factr-I$/i.test(dir) || /[\\/]factr-desktop$/i.test(dir)) {
      return dir
    }

    return null
  }

  // Linux: an AppImage exposes its own path via the APPIMAGE env var.
  if (env.APPIMAGE) {
    return env.APPIMAGE
  }

  // Unpacked electron-builder tree: …/linux-unpacked/Factr-I
  const dir = p.dirname(exe)

  if (/-unpacked$/.test(dir)) {
    return dir
  }

  return null
}

/**
 * Should we even try to remove the running app bundle from a cleanup script?
 * Only when packaged AND we resolved a concrete removable path. Dev runs
 * (electron from node_modules) and system-package installs return null above
 * and are left to the OS package manager.
 */
function shouldRemoveAppBundle(isPackaged, appPath) {
  return Boolean(isPackaged) && Boolean(appPath)
}

/**
 * The packaged app's own interpreter for the uninstaller: the bundled runtime under
 * `<resourcesPath>/backend-python`, with the bundled backend source and packages on PYTHONPATH.
 * A packaged install has no `~/.factr/factr-backend/venv`, so this is what it must use.
 */
function bundledUninstallPython(resourcesPath, platform) {
  const p = platform === 'win32' ? path.win32 : path.posix
  const root = p.join(String(resourcesPath || ''), 'backend-python')

  return {
    pythonExe: p.join(root, 'runtime', platform === 'win32' ? 'python.exe' : p.join('bin', 'python3.12')),
    pythonPath: [p.join(root, 'source'), p.join(root, 'packages')].join(platform === 'win32' ? ';' : ':'),
    cwd: p.join(root, 'source')
  }
}

/**
 * Why the running app bundle cannot be removed, or null when it can. The cleanup script runs after
 * the app quits, so a failure there cannot be shown in the UI: catch the known cases up front.
 *   - macOS App Translocation: Gatekeeper runs a quarantined app from a random read-only mount
 *     (`/private/var/folders/.../AppTranslocation/...`); the real copy's location is unknown.
 *   - a mounted disk image (`/Volumes/...` on a read-only DMG).
 *   - a bundle whose parent folder is not writable by this user.
 * `canWrite(dir)` is injected (fs.accessSync W_OK in main.ts) so this stays pure.
 */
function appBundleRemovalBlocker(appPath, platform, canWrite: (dir: string) => boolean) {
  if (!appPath) {
    return null
  }

  const p = platform === 'win32' ? path.win32 : path.posix
  const where = String(appPath)

  if (platform === 'darwin' && /\/AppTranslocation\//.test(where)) {
    return {
      reason: 'translocated',
      message:
        'macOS is running Factr-I from a temporary read-only copy (App Translocation), so it cannot remove itself. ' +
        'Quit Factr-I, move Factr-I.app to the Applications folder, open it from there and uninstall again, ' +
        'or drag Factr-I.app to the Trash.'
    }
  }

  if (platform === 'darwin' && where.startsWith('/Volumes/')) {
    return {
      reason: 'disk-image',
      message:
        `Factr-I is running from a disk image or external volume (${where}), which it cannot remove. ` +
        'Quit Factr-I, eject the disk image, and uninstall the copy in your Applications folder instead.'
    }
  }

  if (!canWrite(p.dirname(where))) {
    return {
      reason: 'not-writable',
      message:
        `Factr-I cannot delete ${where}: its folder is not writable by your user. ` +
        'Remove it yourself (on macOS, drag it to the Trash and enter an administrator password).'
    }
  }

  return null
}

type UninstallTargetKind = 'agent' | 'app' | 'artifact' | 'config' | 'userdata'

interface UninstallTarget {
  kind: UninstallTargetKind
  path: string
}

interface UninstallRefusal extends UninstallTarget {
  reason: 'contains-kept-data' | 'not-absolute' | 'protected' | 'symlink'
  message: string
}

interface UninstallPlan {
  mode: string
  /** Exactly what the confirmation lists and the cleanup may delete: nothing else. */
  targets: UninstallTarget[]
  /** Targets this mode would have removed but must not (shown, and the run refuses). */
  refused: UninstallRefusal[]
  /** Why the running app bundle cannot remove itself (translocated, disk image, read-only). */
  appBlocked: { reason: string; message: string } | null
}

interface UninstallPlanInput {
  mode: string
  platform: string
  execPath: string
  env?: Record<string, string | undefined>
  isPackaged: boolean
  /** app.getPath('userData') of THIS running copy (FACTR_DESKTOP_USER_DATA_DIR already applied). */
  userDataDir: string
  /** The FACTR_CONFIG_HOME this copy actually runs with. */
  configHome: string
  homeDir: string
  /** A source checkout's agent root that lite/full remove; null for a packaged app (its agent is in the bundle). */
  agentRoot?: null | string
  /** Built GUI artifacts the Python probe found (source installs only). */
  sourceBuiltArtifacts?: string[]
  canWrite: (dir: string) => boolean
  isSymlink: (target: string) => boolean
  /** Fully resolved path (symlinks followed), or the input when it does not exist. */
  realpath?: (target: string) => string
}

/** Folders no uninstall may delete, nor any folder that contains one of them. */
function protectedPaths(platform: string, homeDir: string, env: Record<string, string | undefined>): string[] {
  const p = platform === 'win32' ? path.win32 : path.posix
  const home = homeDir ? p.resolve(homeDir) : ''
  const underHome = (...parts: string[]) => (home ? [p.join(home, ...parts)] : [])

  if (platform === 'win32') {
    const drive = home ? p.parse(home).root : 'C:\\'

    return [
      drive,
      p.join(drive, 'Users'),
      p.join(drive, 'Windows'),
      p.join(drive, 'Program Files'),
      p.join(drive, 'Program Files (x86)'),
      ...(home ? [home] : []),
      ...underHome('AppData'),
      ...underHome('AppData', 'Roaming'),
      ...underHome('AppData', 'Local'),
      ...underHome('AppData', 'Local', 'Programs'),
      ...underHome('Documents'),
      ...underHome('Desktop'),
      ...[env.APPDATA, env.LOCALAPPDATA, env.ProgramFiles].filter((v): v is string => Boolean(v))
    ]
  }

  return [
    '/',
    '/Applications',
    '/Users',
    '/System',
    '/Library',
    '/Volumes',
    '/home',
    '/usr',
    '/bin',
    '/etc',
    '/opt',
    '/private',
    '/tmp',
    '/var',
    ...(home ? [home] : []),
    ...underHome('Applications'),
    ...underHome('Library'),
    ...underHome('Library', 'Application Support'),
    ...underHome('Documents'),
    ...underHome('Desktop'),
    ...underHome('.config'),
    ...underHome('.local'),
    ...underHome('.local', 'share')
  ]
}

/** True when `parent` is `child` or one of its ancestors. */
function isSameOrAncestor(parent: string, child: string, platform: string): boolean {
  const p = platform === 'win32' ? path.win32 : path.posix
  const norm = (value: string) => (platform === 'win32' ? p.resolve(value).toLowerCase() : p.resolve(value))
  const a = norm(parent)
  const b = norm(child)

  if (a === b) {
    return true
  }

  const rel = p.relative(a, b)

  return Boolean(rel) && !rel.startsWith('..') && !p.isAbsolute(rel)
}

/** Why `target` must not be deleted, or null when it is safe to remove. */
function unsafeRemovalReason(
  target: string,
  input: Pick<UninstallPlanInput, 'env' | 'homeDir' | 'isSymlink' | 'platform' | 'realpath'>
): Pick<UninstallRefusal, 'message' | 'reason'> | null {
  const p = input.platform === 'win32' ? path.win32 : path.posix

  if (!target || !p.isAbsolute(target)) {
    return { reason: 'not-absolute', message: `Refusing to delete "${target}": it is not an absolute path.` }
  }

  if (input.isSymlink(target)) {
    return {
      reason: 'symlink',
      message: `Refusing to delete ${target}: it is a link to somewhere else. Remove it yourself if you mean to.`
    }
  }

  const real = input.realpath ? input.realpath(target) : target
  const guarded = protectedPaths(input.platform, input.homeDir, input.env ?? {})

  for (const candidate of new Set([p.resolve(target), real])) {
    if (guarded.some(dir => isSameOrAncestor(candidate, dir, input.platform))) {
      return {
        reason: 'protected',
        message: `Refusing to delete ${target}: it is (or contains) a system or home folder.`
      }
    }
  }

  return null
}

/**
 * The ONE place that decides what an uninstall mode deletes, for the summary the
 * confirmation renders AND for the run, so the two can never disagree:
 *   - the running app bundle (from process.execPath; packaged builds only; a
 *     translocated / disk-image / read-only copy is reported in `appBlocked`),
 *   - this copy's real Electron userData folder (never a default location),
 *   - gui/lite: built GUI artifacts of a source install; lite/full: a source
 *     checkout's agent root; full: the config home this copy runs with.
 * Each target is vetted: an absolute, non-symlink path that is not (and does
 * not contain) a system or home folder. gui/lite promise to keep the config
 * home, so a userData folder that contains it is refused, not deleted.
 */
function resolveUninstallPlan(input: UninstallPlanInput): UninstallPlan {
  const { mode, platform } = input
  const candidates: UninstallTarget[] = []
  let appBlocked: UninstallPlan['appBlocked'] = null

  const appPath = resolveRemovableAppPath(input.execPath, platform, input.env ?? {})

  if (shouldRemoveAppBundle(input.isPackaged, appPath)) {
    appBlocked = appBundleRemovalBlocker(appPath, platform, input.canWrite)
    candidates.push({ kind: 'app', path: String(appPath) })
  }

  for (const artifact of input.sourceBuiltArtifacts ?? []) {
    candidates.push({ kind: 'artifact', path: artifact })
  }

  if (modeRemovesAgent(mode) && input.agentRoot) {
    candidates.push({ kind: 'agent', path: input.agentRoot })
  }

  if (input.userDataDir) {
    candidates.push({ kind: 'userdata', path: input.userDataDir })
  }

  if (modeRemovesUserData(mode) && input.configHome) {
    candidates.push({ kind: 'config', path: input.configHome })
  }

  const targets: UninstallTarget[] = []
  const refused: UninstallRefusal[] = []

  for (const target of candidates) {
    const unsafe = unsafeRemovalReason(target.path, input)

    if (unsafe) {
      refused.push({ ...target, ...unsafe })

      continue
    }

    // gui/lite keep config, chats and secrets: never delete a folder holding them.
    if (!modeRemovesUserData(mode) && input.configHome && isSameOrAncestor(target.path, input.configHome, platform)) {
      refused.push({
        ...target,
        reason: 'contains-kept-data',
        message: `Refusing to delete ${target.path}: it holds your Factr-I config and chats (${input.configHome}), which this option keeps.`
      })

      continue
    }

    if (!targets.some(existing => isSameOrAncestor(existing.path, target.path, platform))) {
      targets.push(target)
    }
  }

  return { mode, targets, refused, appBlocked }
}

/**
 * Build a POSIX cleanup shell script (macOS / Linux). It:
 *   1. waits (bounded ~30s) for the desktop PID to exit (venv/bundle unlock),
 *   2. runs the Python uninstall module with the mode,
 *   3. removes the app bundle if one was resolved.
 *
 * `pythonExe` should be a Python OUTSIDE the venv for lite/full (the venv is
 * being deleted); `pythonPath` is prepended to PYTHONPATH so `import factr_backend`
 * resolves from the agent source. `q()` single-quote-escapes for the shell
 * (closes-escapes-reopens any embedded apostrophe), defending against spaces.
 */
/**
 * The plan's targets for the Python uninstaller, as one env value with no quotes or
 * shell metacharacters (base64 JSON) so it survives both the POSIX and the cmd.exe
 * script. factr_backend.gui_uninstall deletes only these paths when it is set.
 */
function encodeUninstallTargets(targets: UninstallTarget[] | null | undefined): string {
  return Buffer.from(JSON.stringify({ targets: targets ?? [] }), 'utf8').toString('base64')
}

function buildPosixCleanupScript({
  desktopPid,
  pythonExe,
  pythonPath,
  agentRoot,
  uninstallArgs,
  appPath,
  factrHome,
  targets = null
}: {
  desktopPid: number
  pythonExe: string
  pythonPath: null | string
  agentRoot: string
  uninstallArgs: string[]
  appPath: null | string
  factrHome: string
  targets?: null | UninstallTarget[]
}) {
  const q = s => `'${String(s).replace(/'/g, `'\\''`)}'`

  const lines = [
    '#!/bin/bash',
    'set -u',
    '# Wait (up to ~30s) for the desktop process to exit so the venv python',
    '# and the app bundle are no longer in use.',
    `pid=${Number(desktopPid) || 0}`,
    'if [ "$pid" -gt 0 ]; then',
    '  for _ in $(seq 1 60); do',
    '    kill -0 "$pid" 2>/dev/null || break',
    '    sleep 0.5',
    '  done',
    'fi',
    `export FACTR_CONFIG_HOME=${q(factrHome)}`,
    '# Never write bytecode into the (possibly signed) interpreter tree being run.',
    'export PYTHONDONTWRITEBYTECODE=1'
  ]

  if (targets) {
    lines.push(
      '# Delete only what the confirmation listed (factr_backend.gui_uninstall).',
      `export FACTR_UNINSTALL_TARGETS_B64=${q(encodeUninstallTargets(targets))}`
    )
  }

  if (pythonPath) {
    lines.push(`export PYTHONPATH=${q(pythonPath)}\${PYTHONPATH:+:$PYTHONPATH}`)
  }

  lines.push(`cd ${q(agentRoot)} 2>/dev/null || true`, `${q(pythonExe)} ${uninstallArgs.map(q).join(' ')} || true`)

  if (appPath) {
    // The app has quit, so a failed removal cannot reach the UI: say so with a system alert
    // (macOS) or desktop notification (Linux) instead of failing silently.
    const failed = `Factr-I could not remove ${String(appPath)}. Delete it yourself.`
    lines.push(
      `rm -rf ${q(appPath)} 2>/dev/null`,
      `if [ -e ${q(appPath)} ]; then`,
      `  echo ${q(failed)} >&2`,
      '  if command -v osascript >/dev/null 2>&1; then',
      `    osascript -e 'on run argv' -e 'display alert "Uninstall incomplete" message (item 1 of argv) as warning' -e 'end run' ${q(failed)} >/dev/null 2>&1`,
      '  elif command -v notify-send >/dev/null 2>&1; then',
      `    notify-send 'Uninstall incomplete' ${q(failed)} >/dev/null 2>&1`,
      '  fi',
      'fi'
    )
  }

  // Self-delete the script.
  lines.push('rm -f "$0" 2>/dev/null || true')
  lines.push('')

  return lines.join('\n')
}

/**
 * Build a Windows cleanup batch script. Same three steps, cmd.exe flavored.
 *
 * Finding 3 (venv self-deletion): for lite/full the agent uninstall rmtree's
 * the venv that contains `python.exe`. A running .exe is mandatory-locked on
 * Windows, so running the uninstall from the venv's OWN python half-fails. The
 * desktop passes a system Python (findSystemPython) as `pythonExe` for those
 * modes + `pythonPath`=agentRoot so `import factr_backend` resolves from source
 * while the venv is torn down. gui-only doesn't touch the venv, so it can use
 * either interpreter.
 *
 * Wait-loop: bounded (matches POSIX's ~30s cap) so a never-exiting / mismatched
 * PID can't wedge the cleanup forever. The `/FI "PID eq"` filter is an EXACT
 * match, so no redundant `| find` (which would substring-match 99→990).
 *
 * Removal: even after the desktop PID is gone, Windows releases directory
 * handles lazily, so a single `rmdir /s /q` can half-fail — retry up to 10x.
 */
function buildWindowsCleanupScript({
  desktopPid,
  pythonExe,
  pythonPath,
  agentRoot,
  uninstallArgs,
  appPath,
  factrHome,
  targets = null
}: {
  desktopPid: number
  pythonExe: string
  pythonPath: null | string
  agentRoot: string
  uninstallArgs: string[]
  appPath: null | string
  factrHome: string
  targets?: null | UninstallTarget[]
}) {
  const pid = Number(desktopPid) || 0
  // cmd.exe has no string escaping inside quotes; strip embedded quotes (paths
  // under %LOCALAPPDATA% never contain them). `&`/`^` in a path would still be
  // a problem, but Factr install paths don't use them.
  const q = s => `"${String(s).replace(/"/g, '')}"`

  const lines = [
    '@echo off',
    'setlocal enableextensions',
    `set "FACTR_CONFIG_HOME=${String(factrHome).replace(/"/g, '')}"`,
    `set "PID=${pid}"`,
    'set "PYTHONDONTWRITEBYTECODE=1"'
  ]

  if (pythonPath) {
    lines.push(`set "PYTHONPATH=${String(pythonPath).replace(/"/g, '')};%PYTHONPATH%"`)
  }

  if (targets) {
    lines.push(`set "FACTR_UNINSTALL_TARGETS_B64=${encodeUninstallTargets(targets)}"`)
  }

  lines.push(
    'set /a waited=0',
    ':waitloop',
    'rem /FI "PID eq %PID%" is an EXACT filter — tasklist outputs the one task',
    'rem row for that PID, or "INFO: No tasks..." otherwise. /NH drops the',
    'rem header; findstr matches the PID as a whole space-delimited token so',
    'rem PID 99 cannot match 990 (the substring trap of a bare `find`).',
    'tasklist /NH /FI "PID eq %PID%" 2>nul | findstr /r /c:" %PID% " >nul',
    'if %ERRORLEVEL% neq 0 goto waited_done',
    'set /a waited+=1',
    'if %waited% geq 60 goto waited_done',
    'timeout /t 1 /nobreak >nul',
    'goto waitloop',
    ':waited_done',
    `cd /d ${q(agentRoot)}`,
    `${q(pythonExe)} ${uninstallArgs.map(q).join(' ')}`
  )

  if (appPath) {
    lines.push(
      'set /a tries=0',
      ':rmloop',
      `if not exist ${q(appPath)} goto rmdone`,
      `rmdir /s /q ${q(appPath)} >nul 2>&1`,
      `if not exist ${q(appPath)} goto rmdone`,
      'set /a tries+=1',
      'if %tries% geq 10 goto rmdone',
      'timeout /t 1 /nobreak >nul',
      'goto rmloop',
      ':rmdone'
    )
  }

  lines.push('del "%~f0"')
  lines.push('')

  return lines.join('\r\n')
}

export type { UninstallPlan, UninstallPlanInput, UninstallRefusal, UninstallTarget, UninstallTargetKind }

export {
  appBundleRemovalBlocker,
  buildPosixCleanupScript,
  buildWindowsCleanupScript,
  bundledUninstallPython,
  encodeUninstallTargets,
  modeRemovesAgent,
  modeRemovesUserData,
  resolveRemovableAppPath,
  resolveUninstallPlan,
  shouldRemoveAppBundle,
  UNINSTALL_MODES,
  uninstallArgsForMode
}
