/**
 * Tests for electron/desktop-uninstall.ts.
 *
 * Run with: node --test electron/desktop-uninstall.test.ts
 * (Wired into npm test:desktop:platforms in package.json.)
 *
 * These are the pure helpers behind the desktop Chat GUI uninstaller: the
 * mode → CLI-flag mapping, the running-app-bundle resolution per OS, and the
 * cleanup-script builders (POSIX + Windows).
 */

import assert from 'node:assert/strict'

import { test } from 'vitest'

import {
  appBundleRemovalBlocker,
  buildPosixCleanupScript,
  buildWindowsCleanupScript,
  bundledUninstallPython,
  modeRemovesAgent,
  modeRemovesUserData,
  resolveRemovableAppPath,
  shouldRemoveAppBundle,
  uninstallArgsForMode
} from './desktop-uninstall'

// --- uninstallArgsForMode ---

test('uninstallArgsForMode throws on an unknown mode (no silent full wipe)', () => {
  assert.throws(() => uninstallArgsForMode('nuke'), /Unknown uninstall mode/)
  assert.throws(() => uninstallArgsForMode(''), /Unknown uninstall mode/)
})

// --- modeRemovesAgent / modeRemovesUserData ---

test('mode predicates classify what each mode removes', () => {
  assert.equal(modeRemovesAgent('gui'), false)
  assert.equal(modeRemovesAgent('lite'), true)
  assert.equal(modeRemovesAgent('full'), true)

  assert.equal(modeRemovesUserData('gui'), false)
  assert.equal(modeRemovesUserData('lite'), false)
  assert.equal(modeRemovesUserData('full'), true)
})

// --- resolveRemovableAppPath ---

test('resolveRemovableAppPath finds the .app bundle on macOS', () => {
  assert.equal(
    resolveRemovableAppPath('/Applications/Factr.app/Contents/MacOS/Factr', 'darwin'),
    '/Applications/Factr.app'
  )
  assert.equal(
    resolveRemovableAppPath('/Users/x/Applications/Factr.app/Contents/MacOS/Factr', 'darwin'),
    '/Users/x/Applications/Factr.app'
  )
})

test('resolveRemovableAppPath: dev-run .app resolves (safety is shouldRemoveAppBundle, not null)', () => {
  // A dev run from node_modules' Electron DOES resolve to a .app — the real
  // dev-run safety gate is shouldRemoveAppBundle(isPackaged=false,...), not a
  // null return here. This test documents that contract.
  assert.equal(
    resolveRemovableAppPath('/repo/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron', 'darwin'),
    '/repo/node_modules/electron/dist/Electron.app'
  )
  assert.equal(shouldRemoveAppBundle(false, '/repo/node_modules/electron/dist/Electron.app'), false)
  // A bare path with no .app ancestor → null.
  assert.equal(resolveRemovableAppPath('/usr/bin/electron', 'darwin'), null)
})

test('resolveRemovableAppPath finds the install dir on Windows', () => {
  assert.equal(
    resolveRemovableAppPath('C:\\Users\\x\\AppData\\Local\\Programs\\Factr-I\\Factr-I.exe', 'win32'),
    'C:\\Users\\x\\AppData\\Local\\Programs\\Factr-I'
  )
  assert.equal(
    resolveRemovableAppPath('C:\\Users\\x\\AppData\\Local\\factr-desktop\\Factr.exe', 'win32'),
    'C:\\Users\\x\\AppData\\Local\\factr-desktop'
  )
})

test('resolveRemovableAppPath never matches the engine config home named factr', () => {
  assert.equal(resolveRemovableAppPath('C:\\Users\\x\\AppData\\Local\\factr\\Factr-I.exe', 'win32'), null)
  assert.equal(resolveRemovableAppPath('C:\\Users\\x\\AppData\\Local\\Factr\\Factr.exe', 'win32'), null)
})

test('resolveRemovableAppPath returns null for an unrecognized Windows dir', () => {
  assert.equal(resolveRemovableAppPath('C:\\Temp\\foo\\Factr.exe', 'win32'), null)
})

test('resolveRemovableAppPath uses APPIMAGE on Linux when set', () => {
  assert.equal(
    resolveRemovableAppPath('/tmp/.mount_FactrXXXX/factr', 'linux', { APPIMAGE: '/home/x/Apps/Factr.AppImage' }),
    '/home/x/Apps/Factr.AppImage'
  )
})

test('resolveRemovableAppPath finds the unpacked dir on Linux', () => {
  assert.equal(resolveRemovableAppPath('/opt/factr/linux-unpacked/factr', 'linux', {}), '/opt/factr/linux-unpacked')
  // A system-package install (/usr/bin) → null, left to apt/dnf.
  assert.equal(resolveRemovableAppPath('/usr/bin/factr', 'linux', {}), null)
})

test('resolveRemovableAppPath returns null for an empty exe path', () => {
  assert.equal(resolveRemovableAppPath('', 'darwin'), null)
  assert.equal(resolveRemovableAppPath(null, 'win32'), null)
})

// --- shouldRemoveAppBundle ---

test('shouldRemoveAppBundle requires packaged AND a resolved path', () => {
  assert.equal(shouldRemoveAppBundle(true, '/Applications/Factr.app'), true)
  assert.equal(shouldRemoveAppBundle(false, '/Applications/Factr.app'), false)
  assert.equal(shouldRemoveAppBundle(true, null), false)
  assert.equal(shouldRemoveAppBundle(false, null), false)
})

// --- buildPosixCleanupScript ---

test('buildPosixCleanupScript waits for the PID, runs the uninstall module, removes bundle', () => {
  const script = buildPosixCleanupScript({
    desktopPid: 4321,
    pythonExe: '/home/x/.factr/factr-backend/venv/bin/python',
    pythonPath: null,
    agentRoot: '/home/x/.factr/factr-backend',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'gui'],
    appPath: '/opt/factr/linux-unpacked',
    factrHome: '/home/x/.factr'
  })

  assert.match(script, /^#!\/bin\/bash/)
  assert.match(script, /pid=4321/)
  assert.match(script, /kill -0 "\$pid"/)
  assert.match(script, /'-m' 'factr_backend\.uninstall' '--mode' 'gui'/)
  assert.match(script, /rm -rf '\/opt\/factr\/linux-unpacked'/)
  assert.match(script, /export FACTR_CONFIG_HOME='\/home\/x\/\.factr'/)
})

test('buildPosixCleanupScript exports PYTHONPATH when pythonPath is set (lite/full)', () => {
  const script = buildPosixCleanupScript({
    desktopPid: 1,
    pythonExe: '/usr/bin/python3',
    pythonPath: '/home/x/.factr/factr-backend',
    agentRoot: '/home/x/.factr/factr-backend',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'full'],
    appPath: null,
    factrHome: '/home/x/.factr'
  })

  // System python + source on PYTHONPATH so import factr_backend works while the
  // venv is torn down.
  assert.match(script, /export PYTHONPATH='\/home\/x\/\.factr\/factr-backend'/)
  assert.match(script, /'\/usr\/bin\/python3' '-m' 'factr_backend\.uninstall' '--mode' 'full'/)
})

test('buildPosixCleanupScript omits PYTHONPATH when pythonPath is null (gui)', () => {
  const script = buildPosixCleanupScript({
    desktopPid: 1,
    pythonExe: '/p/python',
    pythonPath: null,
    agentRoot: '/a',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'gui'],
    appPath: null,
    factrHome: '/h'
  })

  assert.doesNotMatch(script, /export PYTHONPATH/)
})

test('buildPosixCleanupScript omits the bundle rm when appPath is null', () => {
  const script = buildPosixCleanupScript({
    desktopPid: 1,
    pythonExe: '/p/python',
    pythonPath: null,
    agentRoot: '/a',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'lite'],
    appPath: null,
    factrHome: '/h'
  })

  assert.doesNotMatch(script, /rm -rf '\//)
  // Still runs the uninstall.
  assert.match(script, /'-m' 'factr_backend\.uninstall' '--mode' 'lite'/)
})

test('buildPosixCleanupScript single-quote-escapes paths with apostrophes', () => {
  const script = buildPosixCleanupScript({
    desktopPid: 1,
    pythonExe: "/home/o'brien/python",
    pythonPath: null,
    agentRoot: '/a',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'gui'],
    appPath: null,
    factrHome: '/h'
  })

  // The apostrophe is closed-escaped-reopened so the shell sees the literal.
  assert.match(script, /'\/home\/o'\\''brien\/python'/)
})

// --- buildWindowsCleanupScript ---

test('buildWindowsCleanupScript waits (bounded) for PID, runs uninstall, rmdir bundle', () => {
  const script = buildWindowsCleanupScript({
    desktopPid: 9988,
    pythonExe: 'C:\\Python313\\python.exe',
    pythonPath: 'C:\\factr',
    agentRoot: 'C:\\factr',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'full'],
    appPath: 'C:\\Users\\x\\AppData\\Local\\Programs\\Factr',
    factrHome: 'C:\\Users\\x\\AppData\\Local\\factr'
  })

  assert.match(script, /@echo off/)
  assert.match(script, /set "PID=9988"/)
  // PYTHONPATH set so a system python can import factr_backend from source.
  assert.match(script, /set "PYTHONPATH=C:\\factr;%PYTHONPATH%"/)
  assert.match(script, /"C:\\Python313\\python.exe" "-m" "factr_backend\.uninstall" "--mode" "full"/)
  // Bounded wait-loop (no infinite loop), whole-token PID match (no substring).
  assert.match(script, /if %waited% geq 60 goto waited_done/)
  assert.match(script, /findstr \/r \/c:" %PID% "/)
  // Removal is a retry loop (Windows releases dir handles lazily).
  assert.match(script, /:rmloop/)
  assert.match(script, /rmdir \/s \/q "C:\\Users\\x\\AppData\\Local\\Programs\\Factr" >nul 2>&1/)
  assert.match(script, /if %tries% geq 10 goto rmdone/)
  assert.match(script, /del "%~f0"/)
})

test('buildWindowsCleanupScript omits PYTHONPATH + rmdir when not needed (gui, no bundle)', () => {
  const script = buildWindowsCleanupScript({
    desktopPid: 2,
    pythonExe: 'C:\\h\\venv\\Scripts\\python.exe',
    pythonPath: null,
    agentRoot: 'C:\\h',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'gui'],
    appPath: null,
    factrHome: 'C:\\h'
  })

  assert.doesNotMatch(script, /rmdir/)
  assert.doesNotMatch(script, /set "PYTHONPATH=/)
})

// --- packaged-app uninstall ---

test('bundledUninstallPython uses the app bundle interpreter with source + packages on PYTHONPATH', () => {
  const mac = bundledUninstallPython('/Applications/My Apps/Factr-I.app/Contents/Resources', 'darwin')

  assert.equal(mac.pythonExe, '/Applications/My Apps/Factr-I.app/Contents/Resources/backend-python/runtime/bin/python3.12')
  assert.equal(
    mac.pythonPath,
    '/Applications/My Apps/Factr-I.app/Contents/Resources/backend-python/source:' +
      '/Applications/My Apps/Factr-I.app/Contents/Resources/backend-python/packages'
  )
  assert.equal(mac.cwd, '/Applications/My Apps/Factr-I.app/Contents/Resources/backend-python/source')

  const win = bundledUninstallPython('C:\\Users\\x\\AppData\\Local\\Programs\\Factr-I\\resources', 'win32')

  assert.equal(win.pythonExe, 'C:\\Users\\x\\AppData\\Local\\Programs\\Factr-I\\resources\\backend-python\\runtime\\python.exe')
  assert.match(win.pythonPath, /backend-python\\source;C:.*backend-python\\packages$/)
})

test('a packaged uninstall script with spaces in the path stays quoted and turns bytecode off', () => {
  const bundled = bundledUninstallPython('/Applications/My Apps/Factr-I.app/Contents/Resources', 'darwin')

  const script = buildPosixCleanupScript({
    desktopPid: 7,
    pythonExe: bundled.pythonExe,
    pythonPath: bundled.pythonPath,
    agentRoot: bundled.cwd,
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'lite'],
    appPath: '/Applications/My Apps/Factr-I.app',
    factrHome: '/Users/x/.factr'
  })

  assert.match(script, /export PYTHONDONTWRITEBYTECODE=1/)
  assert.match(script, /'\/Applications\/My Apps\/Factr-I\.app\/Contents\/Resources\/backend-python\/runtime\/bin\/python3\.12' '-m'/)
  assert.match(script, /export PYTHONPATH='\/Applications\/My Apps\/[^']*source:[^']*packages'/)
  // Python runs before the bundle (and the interpreter inside it) is removed.
  assert.ok(script.indexOf("'-m' 'factr_backend.uninstall'") < script.indexOf('rm -rf'))
})

test('a failed bundle removal is reported, not swallowed', () => {
  const script = buildPosixCleanupScript({
    desktopPid: 1,
    pythonExe: '/p/python',
    pythonPath: null,
    agentRoot: '/a',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'gui'],
    appPath: '/Applications/Factr-I.app',
    factrHome: '/h'
  })

  assert.doesNotMatch(script, /rm -rf '\/Applications\/Factr-I\.app' \|\| true/)
  assert.match(script, /if \[ -e '\/Applications\/Factr-I\.app' \]; then/)
  assert.match(script, /osascript/)
  assert.match(script, /notify-send/)
})

test('the Windows script turns bytecode off too', () => {
  const script = buildWindowsCleanupScript({
    desktopPid: 2,
    pythonExe: 'C:\\p\\python.exe',
    pythonPath: null,
    agentRoot: 'C:\\h',
    uninstallArgs: ['-m', 'factr_backend.uninstall', '--mode', 'gui'],
    appPath: null,
    factrHome: 'C:\\h'
  })

  assert.match(script, /set "PYTHONDONTWRITEBYTECODE=1"/)
})

test('appBundleRemovalBlocker explains translocated, disk-image and read-only bundles', () => {
  const writable = () => true

  assert.equal(appBundleRemovalBlocker('/Applications/Factr-I.app', 'darwin', writable), null)
  assert.equal(appBundleRemovalBlocker(null, 'darwin', writable), null)
  assert.equal(
    appBundleRemovalBlocker('/private/var/folders/ab/T/AppTranslocation/1234-ABCD/d/Factr-I.app', 'darwin', writable)?.reason,
    'translocated'
  )
  assert.equal(appBundleRemovalBlocker('/Volumes/Install Factr-I/Factr-I.app', 'darwin', writable)?.reason, 'disk-image')

  const checked: string[] = []

  const readOnly = (dir: string) => {
    checked.push(dir)

    return false
  }

  const blocked = appBundleRemovalBlocker('/Applications/Factr-I.app', 'darwin', readOnly)

  assert.equal(blocked?.reason, 'not-writable')
  assert.deepEqual(checked, ['/Applications'])
  assert.match(blocked?.message || '', /Trash/)
  assert.equal(
    appBundleRemovalBlocker('C:\\Program Files\\Factr-I', 'win32', () => false)?.reason,
    'not-writable'
  )
})
