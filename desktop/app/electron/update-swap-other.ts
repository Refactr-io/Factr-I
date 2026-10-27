/**
 * Windows and Linux "Update now": replace the installed app with the verified download after the app quits, with a
 * rollback when the new build does not come up healthy. The macOS twin is update-swap.ts and shares its markers
 * (<updatesDir>/<version>/ok, post-update.json) so the new app reports health the same way on every platform.
 *
 * Neither platform ships signed, so there is no Squirrel / electron-updater. Every risky step is guarded and every
 * failure returns { ok: false } with the installed app untouched (the caller then falls back to showing the file).
 *
 *   Windows: copy the install folder aside (robocopy, while the app still runs), then a detached PowerShell helper
 *            waits for every process of the install folder to exit, runs the NSIS installer with /S (silent, per
 *            user), launches the new app, waits for its health marker, and otherwise restores the copy.
 *   Linux:   copy the AppImage next to the running one ($APPIMAGE, same folder so the rename is atomic), then a
 *            detached /bin/sh helper waits for the old process, keeps the old image as a hard link, renames the new
 *            one over it, relaunches, waits for the health marker, and otherwise puts the old image back.
 *
 * Pure of Electron (exec, fs, spawn and quit are injected) so it is unit testable.
 */

import path from 'node:path'

import type { ExecResult, ReadyUpdate, SwapResult } from './update-swap'

export interface OtherSwapFs {
  chmod: (file: string, mode: number) => Promise<void>
  copyFile: (from: string, to: string) => Promise<void>
  exists: (target: string) => Promise<boolean>
  mkdir: (dir: string) => Promise<void>
  remove: (target: string) => Promise<void>
  rename: (from: string, to: string) => Promise<void>
  size: (file: string) => Promise<null | number>
  /** True when the folder exists and the current user can write to it. */
  writable: (dir: string) => Promise<boolean>
  writeFile: (file: string, content: string, mode: number) => Promise<void>
}

export interface OtherSwapDeps {
  /** process.env.APPIMAGE (Linux): the AppImage file the user started. */
  appImage: null | string
  currentVersion: string
  /** app.getPath('exe') */
  execPath: string
  exec: (command: string, args: string[]) => Promise<ExecResult>
  fs: OtherSwapFs
  isBusy: () => boolean
  isPackaged: boolean
  /** Flags of this launch worth repeating after the update (only --no-sandbox). */
  launchArgs: string[]
  log?: (message: string) => void
  /** process.pid of the running app. */
  pid: number
  platform: NodeJS.Platform
  quit: () => void
  spawnDetached: (command: string, args: string[]) => void
  updatesDir: string
}

const WIN_UNINSTALLER = 'Uninstall Factr-I.exe'

/** The folder the NSIS installer put the app in (the one with the uninstaller), or null. */
export async function windowsInstallDir(deps: Pick<OtherSwapDeps, 'execPath' | 'fs'>): Promise<null | string> {
  const dir = path.win32.dirname(deps.execPath)

  return (await deps.fs.exists(path.win32.join(dir, WIN_UNINSTALLER))) ? dir : null
}

/** Why this install cannot be replaced in place, or null when it can. */
export async function otherUnsupported(ready: ReadyUpdate, deps: OtherSwapDeps): Promise<null | string> {
  if (!deps.isPackaged) {
    return 'not a packaged app'
  }

  if (deps.platform === 'win32') {
    if (ready.kind !== 'exe') {
      return 'no installer to run'
    }

    const dir = await windowsInstallDir(deps)

    if (!dir) {
      return 'not an installed copy of the app'
    }

    return (await deps.fs.writable(dir)) ? null : `${dir} is not writable`
  }

  if (deps.platform === 'linux') {
    if (ready.kind !== 'appimage') {
      return 'no AppImage to install'
    }

    if (!deps.appImage || !(await deps.fs.exists(deps.appImage))) {
      return 'not running from an AppImage'
    }

    return (await deps.fs.writable(path.dirname(deps.appImage))) ? null : `${path.dirname(deps.appImage)} is not writable`
  }

  return 'not Windows or Linux'
}

/** Where the next AppImage is staged: a hidden sibling of the running one, so the final rename stays on one volume. */
export const appImageStagePath = (appImage: string, version: string) =>
  path.posix.join(path.posix.dirname(appImage), `.${path.posix.basename(appImage)}.${version}.new`)

/**
 * The Linux helper. Arguments (paths are arguments, never interpolated):
 *   $1 AppImage   $2 staged new image   $3 version   $4 updates dir   $5 old version   $6 old pid   $7.. launch flags
 * Tunables via the environment so tests run in seconds: FACTR_SWAP_PROC_WAIT, FACTR_SWAP_HEALTH_WAIT, and
 * FACTR_SWAP_LAUNCH (a command that replaces "start the AppImage", for tests).
 */
export function buildLinuxSwapScript(): string {
  return `#!/bin/sh
TARGET="$1"; NEW="$2"; VERSION="$3"; UPDATES="$4"; FROM="$5"; OLDPID="$6"
shift 6
PROC_WAIT="\${FACTR_SWAP_PROC_WAIT:-180}"
HEALTH_WAIT="\${FACTR_SWAP_HEALTH_WAIT:-120}"
EXTRA_WAIT="\${FACTR_SWAP_EXTRA_WAIT:-180}"
LOG="$UPDATES/helper.log"
BACKUP="$TARGET.old"
MARKER="$UPDATES/$VERSION/ok"
POST="$UPDATES/post-update.json"
log() { printf '%s %s\\n' "$(date '+%Y-%m-%dT%H:%M:%S')" "$*" >> "$LOG" 2>/dev/null; }
alive() { kill -0 "$OLDPID" 2>/dev/null; }
launch() {
  if [ -n "$FACTR_SWAP_LAUNCH" ]; then
    $FACTR_SWAP_LAUNCH "$TARGET" >/dev/null 2>&1 &
  else
    nohup "$TARGET" "$@" >/dev/null 2>&1 &
  fi
}

mkdir -p "$UPDATES/$VERSION" 2>/dev/null
: > "$UPDATES/$VERSION/attempted"
log "helper start: $TARGET -> $VERSION (from $FROM)"

# 1. wait for the old app to exit
i=0
while alive; do
  i=$((i + 1))
  if [ "$i" -gt $((PROC_WAIT * 2)) ]; then
    log "abort: the old app is still running after \${PROC_WAIT}s; nothing was changed"
    rm -f "$UPDATES/$VERSION/attempted" "$NEW"
    launch "$@"
    exit 1
  fi
  sleep 0.5
done

# 2. keep the old image (a hard link costs nothing; a copy when the filesystem cannot link), then rename over it
rm -f "$BACKUP"
if ! ln "$TARGET" "$BACKUP" 2>/dev/null && ! cp -p "$TARGET" "$BACKUP" 2>/dev/null; then
  log "abort: could not keep the old image; nothing was changed"
  rm -f "$NEW"
  launch "$@"
  exit 1
fi
if ! mv -f "$NEW" "$TARGET"; then
  log "abort: could not move the new image into place; the old image is untouched"
  rm -f "$NEW" "$BACKUP"
  launch "$@"
  exit 1
fi
chmod +x "$TARGET" 2>/dev/null
log "swapped; launching the new app"

# 3. relaunch and wait for the new app to prove it is healthy
rm -f "$MARKER"
printf '{"from":"%s","to":"%s","marker":"%s"}\\n' "$FROM" "$VERSION" "$MARKER" > "$POST"
launch "$@"
NEWPID=$!
i=0
while [ ! -f "$MARKER" ]; do
  i=$((i + 1))
  if [ "$i" -gt $((HEALTH_WAIT * 2)) ]; then
    break
  fi
  sleep 0.5
done

# a slow first launch is not a failure: keep waiting while the new app is alive
if [ ! -f "$MARKER" ] && kill -0 "$NEWPID" 2>/dev/null; then
  log "no health marker yet but the new app is running; waiting up to \${EXTRA_WAIT}s more"
  j=0
  while [ ! -f "$MARKER" ] && kill -0 "$NEWPID" 2>/dev/null && [ "$j" -lt $((EXTRA_WAIT * 2)) ]; do
    j=$((j + 1))
    sleep 0.5
  done
fi

if [ -f "$MARKER" ]; then
  rm -f "$BACKUP"
  log "ok: the new app reported healthy; the backup was removed"
  exit 0
fi

# 4. rollback
log "rollback: no health marker after \${HEALTH_WAIT}s; restoring the old image"
kill "$NEWPID" 2>/dev/null
n=0
while kill -0 "$NEWPID" 2>/dev/null && [ "$n" -lt 20 ]; do n=$((n + 1)); sleep 0.5; done
kill -9 "$NEWPID" 2>/dev/null
rm -f "$POST"
: > "$UPDATES/$VERSION/rolledback"
mv -f "$BACKUP" "$TARGET"
launch "$@"
log "rollback done"
exit 1
`
}

/**
 * The Windows helper (PowerShell 5.1, present on every supported Windows). It reads its settings from a JSON file
 * (-ConfigFile): InstallDir, Installer, Version, UpdatesDir, From, ExeName, OldPid, Backup (1 when a copy of the install
 * folder exists). A JSON file keeps paths with spaces, quotes and non-ASCII characters out of every command line.
 * Tunables via the environment: FACTR_SWAP_PROC_WAIT, FACTR_SWAP_HEALTH_WAIT, FACTR_SWAP_INSTALLER_ARGS (test only).
 */
export function buildWindowsSwapScript(): string {
  return `param([string]$ConfigFile)
$cfg = Get-Content -Raw -Encoding UTF8 -LiteralPath $ConfigFile | ConvertFrom-Json
$InstallDir = [string]$cfg.InstallDir
$Installer = [string]$cfg.Installer
$Version = [string]$cfg.Version
$UpdatesDir = [string]$cfg.UpdatesDir
$From = [string]$cfg.From
$ExeName = [string]$cfg.ExeName
$OldPid = [int]$cfg.OldPid
$Backup = [string]$cfg.Backup
$ErrorActionPreference = 'Continue'
$procWait = if ($env:FACTR_SWAP_PROC_WAIT) { [int]$env:FACTR_SWAP_PROC_WAIT } else { 180 }
$healthWait = if ($env:FACTR_SWAP_HEALTH_WAIT) { [int]$env:FACTR_SWAP_HEALTH_WAIT } else { 120 }
$extraWait = if ($env:FACTR_SWAP_EXTRA_WAIT) { [int]$env:FACTR_SWAP_EXTRA_WAIT } else { 180 }
$Log = Join-Path $UpdatesDir 'helper.log'
$VerDir = Join-Path $UpdatesDir $Version
$Marker = Join-Path $VerDir 'ok'
$Post = Join-Path $UpdatesDir 'post-update.json'
$BackupDir = "$InstallDir.previous"
function Log([string]$m) { Add-Content -LiteralPath $Log -Value ("{0} {1}" -f (Get-Date -Format s), $m) -ErrorAction SilentlyContinue }
function InstallAlive {
  if (Get-Process -Id $OldPid -ErrorAction SilentlyContinue) { return $true }
  $prefix = $InstallDir.TrimEnd('\\') + '\\'
  $hit = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -and $_.ProcessId -ne $PID }
  return [bool]$hit
}
function LaunchApp { Start-Process -FilePath (Join-Path $InstallDir $ExeName) -WorkingDirectory $InstallDir }
function Restore {
  if ($Backup -eq '1' -and (Test-Path -LiteralPath $BackupDir)) {
    Log 'restoring the previous install folder'
    & robocopy $BackupDir $InstallDir /MIR /NFL /NDL /NJH /NJS /NP /R:2 /W:1 | Out-Null
    return ($LASTEXITCODE -lt 8)
  }
  return $false
}

New-Item -ItemType Directory -Force -Path $VerDir | Out-Null
'' | Set-Content -LiteralPath (Join-Path $VerDir 'attempted')
Log "helper start: $InstallDir -> $Version (from $From)"

# 1. wait for the old app, its engine and helpers to exit (they lock the install folder)
$i = 0
while (InstallAlive) {
  $i++
  if ($i -gt ($procWait * 2)) {
    Log "abort: the old app is still running after $procWait s; nothing was changed"
    Remove-Item -LiteralPath (Join-Path $VerDir 'attempted') -Force -ErrorAction SilentlyContinue
    LaunchApp
    exit 1
  }
  Start-Sleep -Milliseconds 500
}

# 2. run the installer silently over the old install (per user, same folder)
Remove-Item -LiteralPath $Marker -Force -ErrorAction SilentlyContinue
$argText = if ($env:FACTR_SWAP_INSTALLER_ARGS) { $env:FACTR_SWAP_INSTALLER_ARGS } else { "/S /D=$InstallDir" }
Log "running the installer"
$proc = Start-Process -FilePath $Installer -ArgumentList $argText -Wait -PassThru
Log "installer exit code $($proc.ExitCode)"
$installed = ($proc.ExitCode -eq 0) -and (Test-Path -LiteralPath (Join-Path $InstallDir $ExeName))
if (-not $installed) {
  Log 'rollback: the installer did not finish cleanly'
  $null = Restore
  New-Item -ItemType File -Force -Path (Join-Path $VerDir 'rolledback') | Out-Null
  LaunchApp
  exit 1
}

# 3. launch the new app and wait for it to prove it is healthy
Set-Content -LiteralPath $Post -Value ('{{"from":"{0}","to":"{1}","marker":"{2}"}}' -f $From, $Version, ($Marker -replace '\\\\', '\\\\')) -Encoding ascii
LaunchApp
$i = 0
while (-not (Test-Path -LiteralPath $Marker)) {
  $i++
  if ($i -gt ($healthWait * 2)) { break }
  Start-Sleep -Milliseconds 500
}
if (-not (Test-Path -LiteralPath $Marker) -and (InstallAlive)) {
  Log "no health marker yet but the new app is running; waiting up to $extraWait s more"
  $j = 0
  while (-not (Test-Path -LiteralPath $Marker) -and (InstallAlive) -and $j -lt ($extraWait * 2)) { $j++; Start-Sleep -Milliseconds 500 }
}
if (Test-Path -LiteralPath $Marker) {
  Log 'ok: the new app reported healthy; removing the backup'
  Remove-Item -LiteralPath $BackupDir -Recurse -Force -ErrorAction SilentlyContinue
  Log 'backup removed'
  exit 0
}

# 4. rollback: the new app did not report healthy in time
Log "rollback: no health marker after $healthWait s"
Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
  Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith(($InstallDir.TrimEnd('\\') + '\\'), [StringComparison]::OrdinalIgnoreCase) } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Seconds 2
Remove-Item -LiteralPath $Post -Force -ErrorAction SilentlyContinue
$null = Restore
New-Item -ItemType File -Force -Path (Join-Path $VerDir 'rolledback') | Out-Null
LaunchApp
Log 'rollback done'
exit 1
`
}

/**
 * Starts the PowerShell helper from a tiny batch file. Node's direct spawn of powershell.exe does not survive the app quitting
 * on Windows (measured on a real runner: only `cmd /c start /B powershell ...` runs on after its parent exits), and a batch
 * file with %~dp0 needs no quoting of the folder it sits in.
 */
export const buildWindowsLauncher = (version: string) =>
  `@echo off\r\nstart "" /B powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0swap-${version}.ps1" -ConfigFile "%~dp0swap-${version}.json"\r\n`

/**
 * Windows: keep a copy of the install folder (while the app still runs), write the helper and hand over.
 * Linux: stage the AppImage next to the running one, write the helper and hand over.
 * Returns { ok: true } only after the helper was started and the app asked to quit; on { ok: false } nothing about
 * the installed app was touched.
 */
export async function installOtherAndRestart(ready: ReadyUpdate, deps: OtherSwapDeps): Promise<SwapResult> {
  const log = deps.log ?? (() => {})

  if (deps.isBusy()) {
    log('update swap skipped: an agent turn is running')

    return { ok: false, reason: 'an agent turn is running' }
  }

  const bad = await otherUnsupported(ready, deps)

  if (bad) {
    log(`update swap skipped: ${bad}`)

    return { ok: false, reason: bad }
  }

  if (!(await deps.fs.exists(ready.file))) {
    return { ok: false, reason: 'the downloaded file is missing' }
  }

  try {
    await deps.fs.mkdir(deps.updatesDir)

    if (deps.platform === 'win32') {
      const installDir = (await windowsInstallDir(deps)) as string
      const exeName = path.win32.basename(deps.execPath)
      const script = path.win32.join(deps.updatesDir, `swap-${ready.version}.ps1`)
      const backupDir = `${installDir}.previous`

      // A copy to restore from when the installer or the new app fails; without one the helper still installs.
      await deps.fs.remove(backupDir).catch(() => {})

      const copy = await deps.exec('robocopy', [installDir, backupDir, '/MIR', '/NFL', '/NDL', '/NJH', '/NJS', '/NP', '/R:1', '/W:1'])
      const backedUp = copy.code < 8

      if (!backedUp) {
        await deps.fs.remove(backupDir).catch(() => {})
        log(`update swap: no backup copy (robocopy exit ${copy.code}); installing without one`)
      }

      const config = path.win32.join(deps.updatesDir, `swap-${ready.version}.json`)
      const launcher = path.win32.join(deps.updatesDir, `swap-${ready.version}.cmd`)

      await deps.fs.writeFile(script, buildWindowsSwapScript(), 0o644)
      await deps.fs.writeFile(
        config,
        JSON.stringify({
          InstallDir: installDir,
          Installer: ready.file,
          Version: ready.version,
          UpdatesDir: deps.updatesDir,
          From: deps.currentVersion,
          ExeName: exeName,
          OldPid: deps.pid,
          Backup: backedUp ? '1' : '0'
        }),
        0o644
      )
      await deps.fs.writeFile(launcher, buildWindowsLauncher(ready.version), 0o644)
      log(`update swap: starting the helper for ${ready.version}`)
      deps.spawnDetached('cmd.exe', ['/c', launcher])
      deps.quit()

      return { ok: true }
    }

    const appImage = deps.appImage as string
    const staged = appImageStagePath(appImage, ready.version)
    const wanted = await deps.fs.size(ready.file)

    await deps.fs.remove(staged)
    await deps.fs.copyFile(ready.file, staged)

    if ((await deps.fs.size(staged)) !== wanted) {
      await deps.fs.remove(staged)

      return { ok: false, reason: 'the staged copy is incomplete' }
    }

    await deps.fs.chmod(staged, 0o755)

    const script = path.posix.join(deps.updatesDir, `swap-${ready.version}.sh`)

    await deps.fs.writeFile(script, buildLinuxSwapScript(), 0o755)
    log(`update swap: starting the helper for ${ready.version}`)
    deps.spawnDetached('/bin/sh', [script, appImage, staged, ready.version, deps.updatesDir, deps.currentVersion, String(deps.pid), ...deps.launchArgs])
    deps.quit()

    return { ok: true }
  } catch (error) {
    return { ok: false, reason: error instanceof Error ? error.message : String(error) }
  }
}
