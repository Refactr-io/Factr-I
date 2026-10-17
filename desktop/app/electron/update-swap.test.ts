import { execFileSync, spawn, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  buildSwapScript,
  installAndRestart,
  preflightLaunch,
  readPostUpdate,
  resolveAppBundle,
  stageUpdate,
  stagingDirFor,
  type SwapDeps,
  unsupportedLocation
} from './update-swap'

const tmp: string[] = []
const mk = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'swap-'))
  tmp.push(dir)

  return dir
}
afterEach(() => {
  for (const dir of tmp.splice(0)) {
    fs.rmSync(dir, { recursive: true, force: true })
  }
})

describe('bundle resolution and location rules', () => {
  it('finds the .app from the executable path', () => {
    expect(resolveAppBundle('/Applications/Factr-I.app/Contents/MacOS/Factr-I')).toBe('/Applications/Factr-I.app')
    expect(resolveAppBundle('/usr/local/bin/node')).toBeNull()
  })

  it('refuses disk images and translocated copies', () => {
    expect(unsupportedLocation('/Volumes/Factr-I 0.0.8/Factr-I.app')).toMatch(/disk image/)
    expect(unsupportedLocation('/private/var/folders/x/AppTranslocation/ABC/d/Factr-I.app')).toMatch(/translocated/)
    expect(unsupportedLocation('/Applications/Factr-I.app')).toBeNull()
  })

  it('stages next to the app', () => {
    expect(stagingDirFor('/Applications/Factr-I.app', '0.0.9')).toBe('/Applications/.Factr-I-0.0.9.staging')
  })
})

function baseDeps(over: Partial<SwapDeps> = {}): { deps: SwapDeps; calls: string[] } {
  const calls: string[] = []
  const deps: SwapDeps = {
    execPath: '/Applications/Factr-I.app/Contents/MacOS/Factr-I',
    currentVersion: '0.0.8',
    platform: 'darwin',
    arch: 'arm64',
    isPackaged: true,
    isBusy: () => false,
    updatesDir: '/tmp/none',
    exec: async (cmd, args) => {
      calls.push(`${cmd} ${args[0]}`)

      return { code: 0, stdout: cmd.endsWith('lipo') ? 'arm64' : cmd.endsWith('plutil') ? (args[1] === 'CFBundleShortVersionString' ? '0.0.9' : 'net.refactr.factr') : '', stderr: '' }
    },
    fs: {
      writable: async () => true,
      exists: async target => !target.endsWith('.verified'),
      list: async () => ['Factr-I.app'],
      mkdir: async () => {},
      remove: async () => {},
      writeFile: async () => {}
    },
    spawnDetached: (cmd, args) => void calls.push(`spawn ${cmd} ${path.basename(args[0])}`),
    quit: () => void calls.push('quit')
  }

  return { deps: { ...deps, ...over }, calls }
}
const ready = { file: '/cache/Factr-I-0.0.9-mac-arm64.zip', kind: 'zip' as const, version: '0.0.9' }

describe('installAndRestart preconditions', () => {
  it('stages, verifies and hands over to the helper, then quits', async () => {
    const { deps, calls } = baseDeps()

    expect(await installAndRestart(ready, deps)).toEqual({ ok: true })
    expect(calls.some(c => c.startsWith('/usr/bin/ditto'))).toBe(true)
    expect(calls.some(c => c.startsWith('/usr/bin/codesign'))).toBe(true)
    expect(calls.at(-2)).toBe('spawn /bin/sh swap-0.0.9.sh')
    expect(calls.at(-1)).toBe('quit')
  })

  it('reuses a staging folder that was already verified (no unzip, no signature check)', async () => {
    const { deps, calls } = baseDeps({ fs: { ...baseDeps().deps.fs, exists: async () => true } })

    expect(await installAndRestart(ready, deps)).toEqual({ ok: true })
    expect(calls.some(c => c.startsWith('/usr/bin/ditto'))).toBe(false)
    expect(calls.some(c => c.startsWith('/usr/bin/codesign'))).toBe(false)
    expect(calls.at(-1)).toBe('quit')
  })

  for (const [name, over, reason] of [
    ['an agent turn is running', { isBusy: () => true }, /turn is running/],
    ['not macOS', { platform: 'win32' as const }, /not macOS/],
    ['a dev build', { isPackaged: false }, /not a packaged/],
    ['a DMG volume', { execPath: '/Volumes/X/Factr-I.app/Contents/MacOS/Factr-I' }, /disk image/],
    ['a read-only folder', { fs: { ...baseDeps().deps.fs, writable: async () => false } }, /not writable/],
    ['a missing download', { fs: { ...baseDeps().deps.fs, exists: async () => false } }, /missing/]
  ] as const) {
    it(`does nothing when: ${name}`, async () => {
      const { deps, calls } = baseDeps(over as Partial<SwapDeps>)
      const result = await installAndRestart(ready, deps)

      expect(result.ok).toBe(false)
      expect((result as { reason: string }).reason).toMatch(reason)
      expect(calls).not.toContain('quit')
    })
  }

  it('refuses a bundle with the wrong id or version and never quits', async () => {
    const wrongId = baseDeps({
      exec: async (cmd, args) => ({ code: 0, stdout: cmd.endsWith('plutil') ? (args[1] === 'CFBundleIdentifier' && args[2] ? 'other.app' : '0.0.9') : '', stderr: '' })
    })
    const wrongVersion = baseDeps({
      exec: async (cmd, args) => ({ code: 0, stdout: cmd.endsWith('plutil') ? (args[1] === 'CFBundleShortVersionString' ? '0.0.7' : 'net.refactr.factr') : '', stderr: '' })
    })

    expect((await installAndRestart(ready, wrongVersion.deps)).ok).toBe(false)
    expect(wrongVersion.calls).not.toContain('quit')
    // identical ids on both sides pass; a differing one fails
    let n = 0
    const idDiffers = baseDeps({
      exec: async (cmd, args) => {
        if (cmd.endsWith('plutil') && args[1] === 'CFBundleIdentifier') {
          return { code: 0, stdout: n++ === 0 ? 'a.b' : 'c.d', stderr: '' }
        }

        return { code: 0, stdout: args[1] === 'CFBundleShortVersionString' ? '0.0.9' : '', stderr: '' }
      }
    })

    expect((await installAndRestart(ready, idDiffers.deps)).ok).toBe(false)
    expect(idDiffers.calls).not.toContain('quit')
    void wrongId
  })

  it('fails closed when the signature check fails', async () => {
    const { deps, calls } = baseDeps({
      exec: async (cmd, args) => ({
        code: cmd.endsWith('codesign') ? 1 : 0,
        stdout: cmd.endsWith('plutil') ? (args[1] === 'CFBundleShortVersionString' ? '0.0.9' : 'net.refactr.factr') : '',
        stderr: 'invalid signature'
      })
    })

    expect((await installAndRestart(ready, deps)).ok).toBe(false)
    expect(calls).not.toContain('quit')
  })
})

describe('architecture and launch preflight', () => {
  it('refuses an update that is not built for this Mac and never quits', async () => {
    const { deps, calls } = baseDeps({
      arch: 'x64',
      exec: async (cmd, args) => ({
        code: 0,
        stdout: cmd.endsWith('lipo') ? 'arm64' : cmd.endsWith('plutil') ? (args[1] === 'CFBundleShortVersionString' ? '0.0.9' : 'net.refactr.factr') : '',
        stderr: ''
      })
    })
    const result = await installAndRestart(ready, deps)

    expect(result.ok).toBe(false)
    expect((result as { reason: string }).reason).toMatch(/not built for this Mac/)
    expect(calls).not.toContain('quit')
  })

  it('writes a skip marker when the launch-time location is not usable, once per version', async () => {
    const written: string[] = []
    const { deps } = baseDeps({
      execPath: '/Volumes/X/Factr-I.app/Contents/MacOS/Factr-I',
      fs: { ...baseDeps().deps.fs, writeFile: async file => void written.push(file) }
    })

    expect(await preflightLaunch(ready, deps)).toBe(false)
    expect(written.some(f => f.endsWith('0.0.9/skipped'))).toBe(true)
    expect(await preflightLaunch(ready, baseDeps().deps)).toBe(true)
  })
})

describe('post-update handoff', () => {
  it('only trusts the record for the version that is running', async () => {
    const rec = JSON.stringify({ from: '0.0.8', to: '0.0.9', marker: '/u/0.0.9/ok' })

    expect(await readPostUpdate('/u', '0.0.9', async () => rec)).toEqual({ from: '0.0.8', to: '0.0.9', marker: '/u/0.0.9/ok' })
    expect(await readPostUpdate('/u', '0.0.8', async () => rec)).toBeNull()
    expect(await readPostUpdate('/u', '0.0.9', async () => '{broken')).toBeNull()
    expect(await readPostUpdate('/u', '0.0.9', async () => null)).toBeNull()
  })
})

// The real helper script against fake apps in a temp folder (macOS only: uses pgrep, sh, mv).
describe.skipIf(process.platform !== 'darwin')('the swap helper script', () => {
  function setup() {
    const root = mk()
    const apps = path.join(root, 'Applications')
    const updates = path.join(root, 'updates')
    fs.mkdirSync(path.join(apps, 'Factr-I.app', 'Contents'), { recursive: true })
    fs.writeFileSync(path.join(apps, 'Factr-I.app', 'Contents', 'version'), 'old')
    const staged = path.join(apps, '.Factr-I-0.0.9.staging', 'Factr-I.app')
    fs.mkdirSync(path.join(staged, 'Contents'), { recursive: true })
    fs.writeFileSync(path.join(staged, 'Contents', 'version'), 'new')
    fs.mkdirSync(updates, { recursive: true })
    const script = path.join(updates, 'swap.sh')
    fs.writeFileSync(script, buildSwapScript(), { mode: 0o755 })

    return { root, apps, updates, staged, script, old: path.join(apps, 'Factr-I.app') }
  }
  const fakeOpen = (dir: string, behaviour: 'healthy' | 'broken') => {
    const bin = path.join(dir, 'fake-open.sh')
    // The "app" reads its own version file; a healthy new app writes the marker named in post-update.json.
    fs.writeFileSync(
      bin,
      `#!/bin/sh
APP="$2"
V=$(cat "$APP/Contents/version")
echo "$V" >> "${dir}/launches.log"
if [ "$V" = "new" ] && [ "${behaviour}" = "healthy" ]; then
  M=$(sed -n 's/.*"marker":"\\([^"]*\\)".*/\\1/p' "${dir}/updates/post-update.json")
  mkdir -p "$(dirname "$M")"; : > "$M"
fi
exit 0
`,
      { mode: 0o755 }
    )

    return bin
  }
  const run = (s: ReturnType<typeof setup>, open: string, env: Record<string, string> = {}) =>
    spawnSync('/bin/sh', [s.script, s.old, s.staged, '0.0.9', s.updates, '0.0.8'], {
      env: { ...process.env, FACTR_SWAP_OPEN: open, FACTR_SWAP_PROC_WAIT: '2', FACTR_SWAP_HEALTH_WAIT: '3', ...env },
      encoding: 'utf8'
    })

  it('syntax is valid sh', () => {
    const s = setup()

    expect(() => execFileSync('/bin/sh', ['-n', s.script])).not.toThrow()
  })

  it('replaces the app, relaunches it and drops the backup when the new app reports healthy', () => {
    const s = setup()
    const res = run(s, fakeOpen(s.root, 'healthy'))

    expect(res.status).toBe(0)
    expect(fs.readFileSync(path.join(s.old, 'Contents', 'version'), 'utf8')).toBe('new')
    expect(fs.existsSync(path.join(s.apps, '.Factr-I.app.old'))).toBe(false)
    expect(fs.existsSync(path.join(s.apps, '.Factr-I-0.0.9.staging'))).toBe(false)
    expect(fs.readFileSync(path.join(s.root, 'launches.log'), 'utf8').trim()).toBe('new')
    expect(fs.readFileSync(path.join(s.updates, 'helper.log'), 'utf8')).toMatch(/ok: the new app reported healthy/)
  })

  it('puts the old app back and relaunches it when the new app never reports healthy', () => {
    const s = setup()
    const res = run(s, fakeOpen(s.root, 'broken'))

    expect(res.status).toBe(1)
    expect(fs.readFileSync(path.join(s.old, 'Contents', 'version'), 'utf8')).toBe('old')
    expect(fs.existsSync(path.join(s.apps, '.Factr-I.app.old'))).toBe(false)
    expect(fs.existsSync(path.join(s.updates, 'post-update.json'))).toBe(false)
    expect(fs.existsSync(path.join(s.updates, '0.0.9', 'rolledback'))).toBe(true)
    expect(fs.readFileSync(path.join(s.root, 'launches.log'), 'utf8').trim().split('\n')).toEqual(['new', 'old'])
  })

  it('keeps waiting while the new app is alive, so a slow first launch is not rolled back', () => {
    const s = setup()
    const slow = path.join(s.root, 'slow-open.sh')

    // the "new app" is a process whose command line contains the bundle path; it writes the marker after 4 s
    fs.writeFileSync(
      slow,
      `#!/bin/sh
APP="$2"
echo "$(cat "$APP/Contents/version")" >> "${s.root}/launches.log"
M=$(sed -n 's/.*"marker":"\\([^"]*\\)".*/\\1/p' "${s.updates}/post-update.json")
mkdir -p "$(dirname "$M")"
nohup /bin/sh -c 'sleep 4; : > "$1"' "$APP/Contents/MacOS/x" "$M" >/dev/null 2>&1 &
exit 0
`,
      { mode: 0o755 }
    )

    const res = run(s, slow, { FACTR_SWAP_HEALTH_WAIT: '1', FACTR_SWAP_EXTRA_WAIT: '12' })

    expect(res.status).toBe(0)
    expect(fs.readFileSync(path.join(s.old, 'Contents', 'version'), 'utf8')).toBe('new')
    expect(fs.existsSync(path.join(s.updates, '0.0.9', 'rolledback'))).toBe(false)
  })

  it('relaunches the old app when it never exits, and lets a later launch retry', () => {
    const s = setup()
    const holder = spawn('/bin/sh', ['-c', 'sleep 30; :', `${s.old}/Contents/x`], { detached: true, stdio: 'ignore' })

    holder.unref()

    try {
      const res = run(s, fakeOpen(s.root, 'healthy'), { FACTR_SWAP_PROC_WAIT: '1' })

      expect(res.status).toBe(1)
      expect(fs.readFileSync(path.join(s.root, 'launches.log'), 'utf8').trim()).toBe('old')
      expect(fs.existsSync(path.join(s.updates, '0.0.9', 'attempted'))).toBe(false)
    } finally {
      try {
        process.kill(holder.pid as number)
      } catch {
        // gone
      }
    }
  })

  it('handles an app path with regex characters', () => {
    const s = setup()
    const odd = path.join(s.apps, 'Factr-I (2).app')

    fs.renameSync(s.old, odd)
    const res = spawnSync('/bin/sh', [s.script, odd, s.staged, '0.0.9', s.updates, '0.0.8'], {
      env: { ...process.env, FACTR_SWAP_OPEN: fakeOpen(s.root, 'healthy'), FACTR_SWAP_PROC_WAIT: '2', FACTR_SWAP_HEALTH_WAIT: '3' },
      encoding: 'utf8'
    })

    expect(res.status).toBe(0)
    expect(fs.readFileSync(path.join(odd, 'Contents', 'version'), 'utf8')).toBe('new')
  })

  it('changes nothing when the old app is still running', () => {
    const s = setup()
    // a process whose command line contains the old bundle's Contents/ path
    const holder = spawn('/bin/sh', ['-c', 'sleep 30; :', `${s.old}/Contents/x`], { detached: true, stdio: 'ignore' })
    const pid = holder.pid as number

    holder.unref()

    try {
      const res = run(s, fakeOpen(s.root, 'healthy'), { FACTR_SWAP_PROC_WAIT: '1' })

      expect(res.status).toBe(1)
      expect(fs.readFileSync(path.join(s.old, 'Contents', 'version'), 'utf8')).toBe('old')
    } finally {
      try {
        process.kill(pid)
      } catch {
        // already gone
      }
    }
  })
})

describe('stageUpdate', () => {
  it('shares one run between concurrent callers for the same version', async () => {
    const { deps } = baseDeps({ platform: 'linux' })
    const ready = { file: '/tmp/none/x.zip', kind: 'zip' as const, version: '0.0.9' }
    const first = stageUpdate(ready, deps)
    const second = stageUpdate(ready, deps)

    expect(second).toBe(first)
    await first
    expect(stageUpdate(ready, deps)).not.toBe(first)
  })
})
