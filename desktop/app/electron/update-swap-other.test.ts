import { spawn, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  appImageStagePath,
  buildLinuxSwapScript,
  buildWindowsSwapScript,
  installOtherAndRestart,
  type OtherSwapDeps,
  otherUnsupported
} from './update-swap-other'

const tmp: string[] = []

const mk = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'swap-other-'))
  tmp.push(dir)

  return dir
}

afterEach(() => {
  for (const dir of tmp.splice(0)) {
    fs.rmSync(dir, { recursive: true, force: true })
  }
})

function fakeDeps(over: Partial<OtherSwapDeps> = {}, present: string[] = []) {
  const calls: string[] = []
  const files = new Set(present)

  const deps: OtherSwapDeps = {
    appImage: '/home/u/Apps/Factr-I.AppImage',
    currentVersion: '0.0.13',
    execPath: '/tmp/.mount_x/factr-desktop',
    exec: async (command, args) => {
      calls.push(`exec ${command} ${args.join(' ')}`)

      return { code: 0, stdout: '', stderr: '' }
    },
    fs: {
      chmod: async file => void calls.push(`chmod ${file}`),
      copyFile: async (from, to) => {
        calls.push(`copy ${from} -> ${to}`)
        files.add(to)
      },
      exists: async target => files.has(target),
      mkdir: async () => undefined,
      remove: async target => void files.delete(target),
      rename: async () => undefined,
      size: async file => (files.has(file) ? 100 : null),
      writable: async () => true,
      writeFile: async file => void calls.push(`write ${file}`)
    },
    isBusy: () => false,
    isPackaged: true,
    launchArgs: [],
    pid: 4242,
    platform: 'linux',
    quit: () => void calls.push('quit'),
    spawnDetached: (command, args) => void calls.push(`spawn ${command} ${args.join(' ')}`),
    updatesDir: '/home/u/.config/Factr-I/updates',
    ...over
  }

  return { deps, calls }
}

const linuxReady = { file: '/home/u/.config/Factr-I/updates/0.0.14/Factr-I-0.0.14-linux-x86_64.AppImage', kind: 'appimage' as const, version: '0.0.14' }

const winReady = { file: 'C:\\u\\updates\\0.0.14\\Factr-I-0.0.14-win-x64.exe', kind: 'exe' as const, version: '0.0.14' }

describe('which installs can be replaced in place', () => {
  it('Linux needs a packaged AppImage in a writable folder', async () => {
    const ok = fakeDeps({}, ['/home/u/Apps/Factr-I.AppImage'])

    expect(await otherUnsupported(linuxReady, ok.deps)).toBeNull()
    expect(await otherUnsupported(linuxReady, { ...ok.deps, appImage: null })).toMatch(/AppImage/)
    expect(await otherUnsupported(linuxReady, { ...ok.deps, isPackaged: false })).toMatch(/packaged/)
    expect(await otherUnsupported({ ...linuxReady, kind: 'zip' }, ok.deps)).toMatch(/no AppImage/)
    expect(await otherUnsupported(linuxReady, { ...ok.deps, fs: { ...ok.deps.fs, writable: async () => false } })).toMatch(/not writable/)
  })

  it('Windows needs the NSIS install folder (the one with the uninstaller)', async () => {
    const execPath = 'C:\\Users\\u\\AppData\\Local\\Programs\\Factr-I\\Factr-I.exe'
    const installed = fakeDeps({ platform: 'win32', execPath }, ['C:\\Users\\u\\AppData\\Local\\Programs\\Factr-I\\Uninstall Factr-I.exe'])
    const unpacked = fakeDeps({ platform: 'win32', execPath })

    expect(await otherUnsupported(winReady, installed.deps)).toBeNull()
    expect(await otherUnsupported(winReady, unpacked.deps)).toMatch(/not an installed copy/)
  })
})

describe('installOtherAndRestart', () => {
  it('Linux: stages a hidden sibling, starts the helper and quits', async () => {
    const { deps, calls } = fakeDeps({ launchArgs: ['--no-sandbox'] }, ['/home/u/Apps/Factr-I.AppImage', linuxReady.file])

    expect(await installOtherAndRestart(linuxReady, deps)).toEqual({ ok: true })

    const staged = appImageStagePath('/home/u/Apps/Factr-I.AppImage', '0.0.14')

    expect(staged).toBe('/home/u/Apps/.Factr-I.AppImage.0.0.14.new')
    expect(calls).toContain(`copy ${linuxReady.file} -> ${staged}`)
    expect(calls).toContain(`chmod ${staged}`)

    const spawned = calls.find(c => c.startsWith('spawn'))

    expect(spawned).toContain('/bin/sh')
    expect(spawned).toContain(`/home/u/Apps/Factr-I.AppImage ${staged} 0.0.14`)
    expect(spawned).toMatch(/0\.0\.13 4242 --no-sandbox$/)
    expect(calls.at(-1)).toBe('quit')
  })

  it('Windows: keeps a copy of the install folder, starts the helper and quits', async () => {
    const execPath = 'C:\\Users\\u\\AppData\\Local\\Programs\\Factr-I\\Factr-I.exe'
    const dir = 'C:\\Users\\u\\AppData\\Local\\Programs\\Factr-I'
    const { deps, calls } = fakeDeps({ platform: 'win32', execPath, updatesDir: 'C:\\u\\updates' }, [`${dir}\\Uninstall Factr-I.exe`, winReady.file])

    expect(await installOtherAndRestart(winReady, deps)).toEqual({ ok: true })
    expect(calls.some(c => c.startsWith('exec robocopy') && c.includes(`${dir}.previous`))).toBe(true)

    const spawned = calls.find(c => c.startsWith('spawn')) as string

    expect(spawned).toContain('powershell.exe')
    expect(spawned).toContain(`-InstallDir ${dir}`)
    expect(spawned).toContain('-Backup 1')
    expect(spawned).toContain('-ExeName Factr-I.exe')
    expect(calls.at(-1)).toBe('quit')
  })

  it('Windows: a failed backup copy still installs, without a rollback copy', async () => {
    const execPath = 'C:\\Users\\u\\AppData\\Local\\Programs\\Factr-I\\Factr-I.exe'

    const { deps, calls } = fakeDeps(
      { platform: 'win32', execPath, exec: async () => ({ code: 16, stdout: '', stderr: 'disk full' }) },
      ['C:\\Users\\u\\AppData\\Local\\Programs\\Factr-I\\Uninstall Factr-I.exe', winReady.file]
    )

    expect(await installOtherAndRestart(winReady, deps)).toEqual({ ok: true })
    expect(calls.find(c => c.startsWith('spawn'))).toContain('-Backup 0')
  })

  it('touches nothing while an agent turn runs, when unsupported, or when the file is gone', async () => {
    const busy = fakeDeps({ isBusy: () => true }, ['/home/u/Apps/Factr-I.AppImage', linuxReady.file])

    expect(await installOtherAndRestart(linuxReady, busy.deps)).toEqual({ ok: false, reason: 'an agent turn is running' })

    const unsupported = fakeDeps({ appImage: null }, [linuxReady.file])

    expect((await installOtherAndRestart(linuxReady, unsupported.deps)).ok).toBe(false)

    const missing = fakeDeps({}, ['/home/u/Apps/Factr-I.AppImage'])

    expect(await installOtherAndRestart(linuxReady, missing.deps)).toEqual({ ok: false, reason: 'the downloaded file is missing' })

    for (const { calls } of [busy, unsupported, missing]) {
      expect(calls.some(c => c.startsWith('spawn') || c === 'quit')).toBe(false)
    }
  })

  it('Linux: an incomplete staged copy is removed and nothing starts', async () => {
    const { deps, calls } = fakeDeps(
      {},
      ['/home/u/Apps/Factr-I.AppImage', linuxReady.file]
    )

    // size of the download is 100; make the copy report a different size
    const original = deps.fs.size

    deps.fs.size = async file => (file.endsWith('.new') ? 7 : original(file))

    expect(await installOtherAndRestart(linuxReady, deps)).toEqual({ ok: false, reason: 'the staged copy is incomplete' })
    expect(calls.some(c => c.startsWith('spawn') || c === 'quit')).toBe(false)
  })
})

/** Runs the real Linux helper against fake AppImages (plain shell scripts), so the swap and rollback are proven for real. */
describe.runIf(process.platform !== 'win32')('the Linux helper script, run for real', () => {
  const FAKE_V1 = '#!/bin/sh\necho v1\n'

  function setup(newBody: string) {
    const root = mk()
    const updates = path.join(root, 'updates')
    const target = path.join(root, 'Factr-I.AppImage')
    const staged = path.join(root, '.Factr-I.AppImage.0.0.14.new')
    const script = path.join(root, 'swap.sh')

    fs.mkdirSync(path.join(updates, '0.0.14'), { recursive: true })
    fs.writeFileSync(target, FAKE_V1, { mode: 0o755 })
    fs.writeFileSync(staged, newBody, { mode: 0o755 })
    fs.writeFileSync(script, buildLinuxSwapScript(), { mode: 0o755 })

    return { root, updates, target, staged, script, marker: path.join(updates, '0.0.14', 'ok') }
  }

  const run = (s: ReturnType<typeof setup>, pid: number, env: Record<string, string> = {}) =>
    spawnSync('/bin/sh', [s.script, s.target, s.staged, '0.0.14', s.updates, '0.0.13', String(pid)], {
      env: { ...process.env, FACTR_SWAP_PROC_WAIT: '3', FACTR_SWAP_HEALTH_WAIT: '3', FACTR_SWAP_EXTRA_WAIT: '1', ...env },
      timeout: 30_000
    })

  it('swaps the image, relaunches it, and keeps the new one once it reports healthy', () => {
    const s = setup('#!/bin/sh\n')

    // the new app proves health by creating the marker the helper waits for
    fs.writeFileSync(s.staged, `#!/bin/sh\necho v2\ntouch "${s.marker}"\n`, { mode: 0o755 })

    const result = run(s, 2_147_483_000)

    expect(result.status).toBe(0)
    expect(fs.readFileSync(s.target, 'utf8')).toContain('echo v2')
    expect(fs.existsSync(`${s.target}.old`)).toBe(false)
    expect(fs.existsSync(s.staged)).toBe(false)
    expect(JSON.parse(fs.readFileSync(path.join(s.updates, 'post-update.json'), 'utf8'))).toMatchObject({ from: '0.0.13', to: '0.0.14' })
    expect(fs.statSync(s.target).mode & 0o111).not.toBe(0)
  })

  it('puts the old image back when the new app never reports healthy', () => {
    const s = setup('#!/bin/sh\nexit 1\n')
    const result = run(s, 2_147_483_000)

    expect(result.status).toBe(1)
    expect(fs.readFileSync(s.target, 'utf8')).toBe(FAKE_V1)
    expect(fs.existsSync(path.join(s.updates, '0.0.14', 'rolledback'))).toBe(true)
    expect(fs.existsSync(path.join(s.updates, 'post-update.json'))).toBe(false)
  })

  it('changes nothing when the old app will not exit', async () => {
    const s = setup('#!/bin/sh\necho v2\n')
    const sleeper = spawn('sleep', ['30'], { stdio: 'ignore' })

    try {
      const result = run(s, sleeper.pid as number, { FACTR_SWAP_PROC_WAIT: '1' })

      expect(result.status).toBe(1)
      expect(fs.readFileSync(s.target, 'utf8')).toBe(FAKE_V1)
      expect(fs.existsSync(s.staged)).toBe(false)
      expect(fs.existsSync(path.join(s.updates, '0.0.14', 'attempted'))).toBe(false)
    } finally {
      sleeper.kill()
    }
  })
})

describe('the Windows helper script', () => {
  const script = buildWindowsSwapScript()

  it('runs the installer silently into the same folder, waits for health, and can restore', () => {
    expect(script).toContain('/S /D=$InstallDir')
    expect(script).toContain('robocopy $BackupDir $InstallDir /MIR')
    expect(script).toContain('rolledback')
    expect(script).toContain('post-update.json')
    expect(script).toContain('InstallAlive')
  })
})

/** On a Windows runner (CI) run the real helper with a fake installer and fake app (.cmd files). */
describe.runIf(process.platform === 'win32')('the Windows helper script, run for real', () => {
  function setup(healthy: boolean) {
    const root = mk()
    const installDir = path.join(root, 'Programs', 'Factr-I')
    const updates = path.join(root, 'updates')
    const marker = path.join(updates, '0.0.14', 'ok')
    const installer = path.join(root, 'fake-installer.cmd')
    const script = path.join(root, 'swap.ps1')

    fs.mkdirSync(installDir, { recursive: true })
    fs.mkdirSync(path.join(updates, '0.0.14'), { recursive: true })
    fs.writeFileSync(path.join(installDir, 'app.cmd'), '@echo off\r\nrem v1\r\n')
    fs.writeFileSync(path.join(root, 'new-app.cmd'), `@echo off\r\nrem v2\r\n${healthy ? `type nul > "${marker}"\r\n` : ''}`)
    // the real installer receives "/S /D=<dir>": %1 is /S, %2 is /D=<dir>
    fs.writeFileSync(installer, '@echo off\r\nset D=%2\r\nset D=%D:~3%\r\ncopy /Y "%~dp0new-app.cmd" "%D%\\app.cmd" >nul\r\nexit /b 0\r\n')
    fs.writeFileSync(script, buildWindowsSwapScript())

    return { root, installDir, updates, installer, script }
  }

  const run = (s: ReturnType<typeof setup>) =>
    spawnSync(
      'powershell.exe',
      [
        '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', s.script, '-InstallDir', s.installDir, '-Installer', s.installer,
        '-Version', '0.0.14', '-UpdatesDir', s.updates, '-From', '0.0.13', '-ExeName', 'app.cmd', '-OldPid', '999999', '-Backup', '1'
      ],
      { env: { ...process.env, FACTR_SWAP_PROC_WAIT: '5', FACTR_SWAP_HEALTH_WAIT: '5', FACTR_SWAP_EXTRA_WAIT: '1' }, timeout: 120_000 }
    )

  it('installs over the old folder and keeps the new version once healthy', () => {
    const s = setup(true)

    // a stale copy exists as the real flow makes one before the helper starts
    fs.cpSync(s.installDir, `${s.installDir}.previous`, { recursive: true })

    const result = run(s)

    expect(result.status, String(result.stderr)).toBe(0)
    expect(fs.readFileSync(path.join(s.installDir, 'app.cmd'), 'utf8')).toContain('v2')
    expect(fs.existsSync(`${s.installDir}.previous`)).toBe(false)
  })

  it('restores the previous folder when the new app never reports healthy', () => {
    const s = setup(false)

    fs.cpSync(s.installDir, `${s.installDir}.previous`, { recursive: true })

    const result = run(s)

    expect(result.status, String(result.stderr)).toBe(1)
    expect(fs.readFileSync(path.join(s.installDir, 'app.cmd'), 'utf8')).toContain('v1')
    expect(fs.existsSync(path.join(s.updates, '0.0.14', 'rolledback'))).toBe(true)
  })
})
