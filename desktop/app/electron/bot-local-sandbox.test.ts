import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
import { afterEach, describe, expect, it } from 'vitest'

import { prepareBotSandbox, sandboxCommand, seatbeltProfile } from './bot-local-sandbox'

const roots: string[] = []
afterEach(() => roots.splice(0).forEach(root => fs.rmSync(root, { recursive: true, force: true })))
function fixture() {
  const configHome = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'bots-sandbox-check-')))
  roots.push(configHome)
  const root = path.join(configHome, 'profiles', 'alpha')
  fs.mkdirSync(root, { recursive: true })
  return { configHome, root }
}

describe('platform confinement plans', () => {
  it('uses Seatbelt with escaped paths and original argv, never a shell string', () => {
    const root = '/Users/person/bot "one"'
    const plan = sandboxCommand(
      'darwin',
      '/app/factr',
      ['--profile', 'alpha', 'serve'],
      root,
      '/private/tmp/own',
      ['/app'],
      () => true
    )
    expect(plan.command).toBe('/usr/bin/sandbox-exec')
    expect(plan.args.slice(-4)).toEqual(['/app/factr', '--profile', 'alpha', 'serve'])
    expect(plan.args[1]).toContain('(deny default)')
    expect(plan.args[1]).toContain('bot \\"one\\"')
    expect(plan.args[1]).not.toContain('(allow default)')
  })
  it('uses Linux user/PID namespaces, private temp and read-only runtime binds', () => {
    const plan = sandboxCommand('linux', '/app/factr', ['serve'], '/data/alpha', '/tmp/own', ['/app'], () => true)
    expect(plan.command).toBe('/usr/bin/bwrap')
    expect(plan.args).toContain('--unshare-pid')
    expect(plan.args).toContain('--die-with-parent')
    expect(plan.args.join(' ')).toContain('--ro-bind /app /app')
    expect(plan.args.join(' ')).toContain('--bind /data/alpha /data/alpha')
    expect(plan.args.slice(-3)).toEqual(['--', '/app/factr', 'serve'])
    expect(plan.args.join(' ')).not.toContain('--bind / /')
  })
  it.each(['darwin', 'linux', 'win32', 'other'])('fails closed without a sandbox on %s', platform => {
    expect(() => sandboxCommand(platform, '/app/factr', [], '/data/a', '/tmp/t', [], () => false)).toThrow(
      'local-sandbox-unavailable'
    )
  })
  it('uses the bundled Windows runtime/helper with a data-only argv manifest', () => {
    const plan = sandboxCommand(
      'win32',
      'C:\\App\\factr.exe',
      ['serve', '--token-stdin'],
      'C:\\Bots\\alpha',
      'C:\\Temp\\own',
      ['C:\\App'],
      () => true,
      { python: 'C:\\App\\python.exe', script: 'C:\\App\\tools\\windows_bot_sandbox.py' }
    )
    expect(plan.command).toBe('C:\\App\\python.exe')
    expect(plan.args.slice(0, 2)).toEqual(['-I', 'C:\\App\\tools\\windows_bot_sandbox.py'])
    expect(JSON.parse(plan.args[2])).toEqual({
      command: 'C:\\App\\factr.exe',
      args: ['serve', '--token-stdin'],
      root: 'C:\\Bots\\alpha',
      temp: 'C:\\Temp\\own',
      reads: ['C:\\App']
    })
    expect(plan.args.join(' ')).not.toContain('powershell')
  })
})

describe.skipIf(process.platform !== 'darwin')('real macOS sandbox', () => {
  it('confines files and descendants, denies sibling/host canaries and symlink escapes', () => {
    const { root, configHome } = fixture()
    const temp = path.join(root, 'tmp')
    fs.mkdirSync(temp)
    const canary = path.join(configHome, 'host-canary')
    fs.writeFileSync(canary, 'only the host may read this')
    fs.symlinkSync(canary, path.join(root, 'escape'))
    const policy = seatbeltProfile(root, temp, [])
    const run = (code: string) =>
      execFileSync('/usr/bin/sandbox-exec', ['-p', policy, '/bin/sh', '-c', code], { cwd: root, encoding: 'utf8' })
    expect(run('echo own > own.txt; cat own.txt')).toBe('own\n')
    expect(run(`/bin/sh -c 'cat "${canary}" 2>/dev/null || echo denied'`)).toBe('denied\n')
    expect(run('cat escape 2>/dev/null || echo denied')).toBe('denied\n')
    expect(run(`(echo overwrite > "${canary}") 2>/dev/null || echo denied`)).toBe('denied\n')
    expect(fs.readFileSync(canary, 'utf8')).toBe('only the host may read this')
  })
  it('gives the engine private state/browser/temp paths and removes inherited browser/socket attachments', () => {
    const { configHome, root } = fixture()
    const launch = prepareBotSandbox({
      profile: 'alpha',
      configHome,
      command: '/bin/sh',
      args: [],
      env: {
        BROWSER_CDP_URL: 'http://host-browser',
        SSH_AUTH_SOCK: '/host/ssh',
        AGENT_BROWSER_EXECUTABLE_PATH: '/host/browser',
        FACTR_REAL_HOME: '/host/home',
        PATH: '/usr/bin:/bin'
      },
      readRoots: []
    })
    try {
      expect(launch.env.FACTR_HOME).toBe(path.join(root, 'engine'))
      expect(launch.env.AGENT_BROWSER_PROFILE).toBe(path.join(root, 'browser-profile'))
      expect(launch.env.BROWSER_CDP_URL).toBeUndefined()
      expect(launch.env.SSH_AUTH_SOCK).toBeUndefined()
      expect(launch.env.AGENT_BROWSER_EXECUTABLE_PATH).toBeUndefined()
      expect(launch.env.FACTR_REAL_HOME).toBe(launch.env.HOME)
      expect(launch.env.PLAYWRIGHT_BROWSERS_PATH).toBe(path.join(root, '.local-computer/browsers'))
      expect(launch.env.FACTR_REPL).toBe('0')
      expect(fs.realpathSync(path.join(launch.env.FACTR_CONFIG_HOME!, 'profiles/alpha'))).toBe(root)
      expect(fs.realpathSync(path.join(launch.env.FACTR_CONFIG_HOME!, 'bot_relay'))).toBe(path.join(root, 'bot_relay'))
    } finally {
      launch.cleanup()
    }
  })
  it('rejects an external profile symlink and a container-owned profile', () => {
    const { configHome, root } = fixture()
    fs.symlinkSync(os.tmpdir(), path.join(configHome, 'profiles', 'outside'))
    fs.symlinkSync(root, path.join(configHome, 'profiles', 'alias'))
    const options = { configHome, command: '/bin/sh', args: [], env: {}, readRoots: [] }
    expect(() => prepareBotSandbox({ ...options, profile: 'outside' })).toThrow('own profile directory')
    expect(() => prepareBotSandbox({ ...options, profile: 'alias' })).toThrow('own profile directory')
    fs.writeFileSync(path.join(root, '.computer'), '{}')
    expect(() => prepareBotSandbox({ ...options, profile: 'alpha' })).toThrow('Docker computer')
  })
  it('rejects a symlinked state ancestor before creating files outside the bot', () => {
    const { configHome, root } = fixture()
    const outside = path.join(configHome, 'outside')
    fs.mkdirSync(outside)
    fs.symlinkSync(outside, path.join(root, '.local-computer'))
    expect(() =>
      prepareBotSandbox({ profile: 'alpha', configHome, command: '/bin/sh', args: [], env: {}, readRoots: [] })
    ).toThrow('outside its sandbox')
    expect(fs.readdirSync(outside)).toEqual([])
  })
})
