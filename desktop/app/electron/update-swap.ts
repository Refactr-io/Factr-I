/**
 * macOS "Restart to update": replace the installed Factr-I.app with the verified new one after the app quits,
 * with a rollback if the new build does not come up healthy.
 *
 * The app is only ad-hoc signed, so Squirrel/electron-updater cannot be used and the swap is hand built. Every
 * risky step is guarded and every failure falls back to "show the downloaded file in Finder" (the caller does that
 * when this returns { ok: false }). Nothing here deletes the old app before the new one is staged, verified and
 * in place, and the old app is kept until the new one has reported healthy.
 *
 *   1. preconditions: packaged mac app in a writable, real install location (not a DMG volume, not App Translocation),
 *      no agent turn running
 *   2. stage: `ditto -x -k` the verified zip next to the app (same volume, so the renames are atomic), then check
 *      the bundle id, the version and `codesign --verify --deep --strict`, and drop the quarantine flag
 *   3. helper: a detached /bin/sh script waits for every process of the old bundle to exit, renames old -> .old and
 *      staged -> app, relaunches, waits for the health marker the new app writes after its backend is ready, and
 *      otherwise puts the old app back and relaunches it
 *
 * Pure of Electron (exec, fs, spawn and quit are injected) so it is unit testable.
 */

import path from 'node:path'

export interface ReadyUpdate {
  file: string
  kind: 'appimage' | 'dmg' | 'exe' | 'zip'
  version: string
}

export interface ExecResult {
  code: number
  stderr: string
  stdout: string
}

export interface SwapFs {
  /** True when the folder exists and the current user can write to it. */
  writable: (dir: string) => Promise<boolean>
  exists: (target: string) => Promise<boolean>
  list: (dir: string) => Promise<string[]>
  mkdir: (dir: string) => Promise<void>
  remove: (target: string) => Promise<void>
  writeFile: (file: string, content: string, mode: number) => Promise<void>
}

export interface SwapDeps {
  /** app.getPath('exe'): <bundle>/Contents/MacOS/<name>. */
  execPath: string
  currentVersion: string
  platform: NodeJS.Platform
  isPackaged: boolean
  isBusy: () => boolean
  updatesDir: string
  exec: (command: string, args: string[]) => Promise<ExecResult>
  fs: SwapFs
  /** Starts the helper fully detached from this process (stdio ignored, unref'd). */
  spawnDetached: (command: string, args: string[]) => void
  quit: () => void
  log?: (message: string) => void
}

export type SwapResult = { ok: true } | { ok: false; reason: string }

/** The path of the .app that contains `execPath`, or null. */
export function resolveAppBundle(execPath: string): null | string {
  const marker = '.app/Contents/MacOS/'
  const index = execPath.indexOf(marker)

  return index < 0 ? null : execPath.slice(0, index + '.app'.length)
}

/** Why this install cannot be swapped in place, or null when it can. */
export function unsupportedLocation(appPath: string): null | string {
  if (appPath.startsWith('/Volumes/')) {
    return 'running from a disk image'
  }

  if (appPath.includes('/AppTranslocation/')) {
    return 'running from a temporary translocated copy'
  }

  if (!appPath.endsWith('.app')) {
    return 'not an app bundle'
  }

  return null
}

export const stagingDirFor = (appPath: string, version: string) =>
  path.join(path.dirname(appPath), `.${path.basename(appPath, '.app')}-${version}.staging`)

/**
 * The detached helper. Arguments (all paths are passed as arguments, never interpolated):
 *   $1 old app   $2 staged app   $3 version   $4 updates dir   $5 old version
 * Tunables via the environment so tests can run it in seconds: FACTR_SWAP_OPEN (default /usr/bin/open),
 * FACTR_SWAP_PROC_WAIT (seconds to wait for the old app to exit, default 60), FACTR_SWAP_HEALTH_WAIT (seconds to
 * wait for the new app's health marker, default 45).
 */
export function buildSwapScript(): string {
  return `#!/bin/sh
OLD="$1"; NEW="$2"; VERSION="$3"; UPDATES="$4"; FROM="$5"
OPEN="\${FACTR_SWAP_OPEN:-/usr/bin/open}"
PROC_WAIT="\${FACTR_SWAP_PROC_WAIT:-60}"
HEALTH_WAIT="\${FACTR_SWAP_HEALTH_WAIT:-45}"
LOG="$UPDATES/helper.log"
BACKUP="$(dirname "$OLD")/.$(basename "$OLD").old"
MARKER="$UPDATES/$VERSION/ok"
POST="$UPDATES/post-update.json"
log() { printf '%s %s\\n' "$(date '+%Y-%m-%dT%H:%M:%S')" "$*" >> "$LOG" 2>/dev/null; }
alive() { pgrep -f "$OLD/Contents/" >/dev/null 2>&1; }

mkdir -p "$UPDATES/$VERSION" 2>/dev/null
log "helper start: $OLD -> $VERSION (from $FROM)"

# 1. wait for the old app and its engine/backend/helpers to exit
i=0
while alive; do
  i=$((i + 1))
  if [ "$i" -gt $((PROC_WAIT * 2)) ]; then
    log "abort: the old app is still running after \${PROC_WAIT}s; nothing was changed"
    rm -rf "$NEW"
    case "$(basename "$(dirname "$NEW")")" in .*.staging) rm -rf "$(dirname "$NEW")" ;; esac
    exit 1
  fi
  sleep 0.5
done

# 2. swap with a restorable backup; never delete the old app first
rm -rf "$BACKUP"
if ! mv "$OLD" "$BACKUP"; then
  log "abort: could not move the old app aside; nothing was changed"
  rm -rf "$NEW"
  "$OPEN" -n "$OLD" >/dev/null 2>&1
  exit 1
fi
if ! mv "$NEW" "$OLD"; then
  log "abort: could not move the new app into place; restoring the old app"
  mv "$BACKUP" "$OLD"
  rm -rf "$NEW"
  "$OPEN" -n "$OLD" >/dev/null 2>&1
  exit 1
fi
STAGING="$(dirname "$NEW")"
case "$(basename "$STAGING")" in .*.staging) rm -rf "$STAGING" ;; esac
log "swapped; launching the new app"

# 3. relaunch and wait for the new app to prove it is healthy
rm -f "$MARKER"
printf '{"from":"%s","to":"%s","marker":"%s"}\\n' "$FROM" "$VERSION" "$MARKER" > "$POST"
"$OPEN" -n "$OLD" >/dev/null 2>&1
i=0
while [ ! -f "$MARKER" ]; do
  i=$((i + 1))
  if [ "$i" -gt $((HEALTH_WAIT * 2)) ]; then
    break
  fi
  sleep 0.5
done

if [ -f "$MARKER" ]; then
  rm -rf "$BACKUP"
  log "ok: the new app reported healthy; the backup was removed"
  exit 0
fi

# 4. rollback: the new app did not report healthy in time
log "rollback: no health marker after \${HEALTH_WAIT}s; restoring the old app"
pkill -f "$OLD/Contents/" >/dev/null 2>&1
n=0
while alive && [ "$n" -lt 20 ]; do n=$((n + 1)); sleep 0.5; done
rm -f "$POST"
rm -rf "$OLD.failed"
: > "$UPDATES/$VERSION/rolledback"
mv "$OLD" "$OLD.failed" && mv "$BACKUP" "$OLD" && rm -rf "$OLD.failed"
"$OPEN" -n "$OLD" >/dev/null 2>&1
log "rollback done"
exit 1
`
}

async function plistValue(deps: SwapDeps, plist: string, key: string): Promise<null | string> {
  const res = await deps.exec('/usr/bin/plutil', ['-extract', key, 'raw', '-o', '-', plist])

  return res.code === 0 ? res.stdout.trim() : null
}

export type StageResult = { ok: false; reason: string } | { ok: true; stagedApp: string }

const VERIFIED = '.verified'

/** Checks every precondition that does not need the download; null when the swap is allowed. */
function precheck(ready: ReadyUpdate, deps: SwapDeps): null | string {
  if (deps.platform !== 'darwin') {return 'not macOS'}
  if (!deps.isPackaged) {return 'not a packaged app'}
  if (ready.kind !== 'zip') {return 'no zip to install from'}

  const appPath = resolveAppBundle(deps.execPath)

  if (!appPath) {return 'cannot locate the app bundle'}

  return unsupportedLocation(appPath)
}

/**
 * Unzip the verified download next to the app and check it, so the swap itself later takes no time. Safe to run
 * in the background right after the download finishes (it never touches the installed app) and idempotent: a
 * staging folder that was already verified is reused.
 */
export async function stageUpdate(ready: ReadyUpdate, deps: SwapDeps): Promise<StageResult> {
  const log = deps.log ?? (() => {})
  const bad = precheck(ready, deps)
  const fail = (reason: string): StageResult => {
    log(`update staging skipped: ${reason}`)

    return { ok: false, reason }
  }

  if (bad) {return fail(bad)}

  const appPath = resolveAppBundle(deps.execPath) as string
  const parent = path.dirname(appPath)

  if (!(await deps.fs.writable(parent))) {return fail(`${parent} is not writable`)}
  if (!(await deps.fs.exists(ready.file))) {return fail('the downloaded file is missing')}

  const staging = stagingDirFor(appPath, ready.version)

  try {
    if (await deps.fs.exists(path.join(staging, VERIFIED))) {
      const apps = (await deps.fs.list(staging)).filter(name => name.endsWith('.app'))

      if (apps.length === 1) {return { ok: true, stagedApp: path.join(staging, apps[0]) }}
    }

    await deps.fs.remove(staging)
    await deps.fs.mkdir(staging)

    const unzip = await deps.exec('/usr/bin/ditto', ['-x', '-k', ready.file, staging])

    if (unzip.code !== 0) {throw new Error(`unzip failed: ${unzip.stderr.trim() || unzip.code}`)}

    const apps = (await deps.fs.list(staging)).filter(name => name.endsWith('.app'))

    if (apps.length !== 1) {throw new Error(`expected one app in the update, found ${apps.length}`)}

    const stagedApp = path.join(staging, apps[0])
    const stagedPlist = path.join(stagedApp, 'Contents', 'Info.plist')
    const currentPlist = path.join(appPath, 'Contents', 'Info.plist')
    const [stagedId, currentId, stagedVersion] = await Promise.all([
      plistValue(deps, stagedPlist, 'CFBundleIdentifier'),
      plistValue(deps, currentPlist, 'CFBundleIdentifier'),
      plistValue(deps, stagedPlist, 'CFBundleShortVersionString')
    ])

    if (!stagedId || stagedId !== currentId) {throw new Error(`bundle id mismatch (${stagedId ?? 'none'} vs ${currentId ?? 'none'})`)}
    if (stagedVersion !== ready.version) {throw new Error(`version mismatch (${stagedVersion ?? 'none'} vs ${ready.version})`)}

    const verify = await deps.exec('/usr/bin/codesign', ['--verify', '--deep', '--strict', stagedApp])

    if (verify.code !== 0) {throw new Error(`code signature check failed: ${verify.stderr.trim() || verify.code}`)}

    // A file this app downloaded itself carries no quarantine flag; clearing it is harmless and makes sure of it.
    await deps.exec('/usr/bin/xattr', ['-dr', 'com.apple.quarantine', stagedApp])
    await deps.fs.writeFile(path.join(staging, VERIFIED), ready.version, 0o644)

    return { ok: true, stagedApp }
  } catch (error) {
    await deps.fs.remove(staging).catch(() => {})

    return fail(error instanceof Error ? error.message : String(error))
  }
}

/**
 * Stage (if not already) and hand over. Returns { ok: true } only after the helper was started and the app asked
 * to quit; on { ok: false } nothing about the installed app was touched.
 */
export async function installAndRestart(ready: ReadyUpdate, deps: SwapDeps): Promise<SwapResult> {
  const log = deps.log ?? (() => {})

  if (deps.isBusy()) {
    log('update swap skipped: an agent turn is running')

    return { ok: false, reason: 'an agent turn is running' }
  }

  const staged = await stageUpdate(ready, deps)

  if (!staged.ok) {return staged}

  const appPath = resolveAppBundle(deps.execPath) as string
  const script = path.join(deps.updatesDir, `swap-${ready.version}.sh`)

  try {
    await deps.fs.mkdir(deps.updatesDir)
    await deps.fs.writeFile(script, buildSwapScript(), 0o755)
    log(`update swap: starting the helper for ${ready.version}`)
    deps.spawnDetached('/bin/sh', [script, appPath, staged.stagedApp, ready.version, deps.updatesDir, deps.currentVersion])
    deps.quit()

    return { ok: true }
  } catch (error) {
    return { ok: false, reason: error instanceof Error ? error.message : String(error) }
  }
}

export interface PostUpdate {
  from: string
  marker: string
  to: string
}

/**
 * Called at startup of the NEW app: reads what the helper left, and returns it only when this really is the
 * version the helper installed. The caller shows "Updated to X" once and writes the marker with markHealthy().
 */
export async function readPostUpdate(
  updatesDir: string,
  currentVersion: string,
  read: (file: string) => Promise<null | string>
): Promise<null | PostUpdate> {
  const raw = await read(path.join(updatesDir, 'post-update.json'))

  if (!raw) {
    return null
  }

  try {
    const parsed = JSON.parse(raw) as Partial<PostUpdate>

    if (typeof parsed.to === 'string' && typeof parsed.from === 'string' && typeof parsed.marker === 'string' && parsed.to === currentVersion) {
      return { from: parsed.from, marker: parsed.marker, to: parsed.to }
    }
  } catch {
    // unreadable: ignore
  }

  return null
}
