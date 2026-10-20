import { describe, expect, it, vi } from 'vitest'

import { createDockerSetup, dockerInstallPlan, hasWsl2 } from './bot-docker-setup'

describe('platform setup plans', () => {
  it('uses Homebrew or the official Mac installer without accepting a license', () => {
    expect(dockerInstallPlan('darwin', () => true).commands).toEqual([{ program: 'brew', args: ['install', '--cask', 'docker'] }])
    expect(dockerInstallPlan('darwin', () => false).download).toBe('https://docs.docker.com/desktop/setup/install/mac-install/')
  })
  it('checks WSL 2 before the interactive Windows installer', () => {
    expect(hasWsl2('WSL version: 2.1.5.0')).toBe(true)
    expect(hasWsl2('Version WSL : 2.0.9')).toBe(false)
    expect(hasWsl2('Windows Subsystem unavailable')).toBe(false)
    expect(dockerInstallPlan('win32', () => true, false).restart).toBe(true)
    const plan = dockerInstallPlan('win32', () => true)
    expect(plan.commands[0].args).toContain('--interactive')
    expect(JSON.stringify(plan)).not.toContain('--accept')
    expect(dockerInstallPlan('win32', () => false).download).toContain('windows-install')
  })
  it('uses the Linux system package manager and leaves other distributions an explicit next step', () => {
    expect(dockerInstallPlan('linux', () => true).commands[1]).toEqual({ program: 'pkexec', args: ['apt-get', 'install', '-y', 'docker.io'] })
    expect(dockerInstallPlan('linux', () => false).download).toContain('/engine/install/')
  })
})

function fixture(platform = 'darwin') {
  let clock = 0, ready = false, installed = false
  const run = vi.fn(async (_program: string, _args: string[], _timeoutMs: number) => { installed = true; ready = true; return '' })
  const confirm = vi.fn(async () => true)
  const deps = {
    platform, available: () => true, run, confirm, openExternal: vi.fn(async () => {}),
    probe: vi.fn(async () => { if (!ready) {throw Object.assign(new Error('not running'), installed ? {} : { code: 'ENOENT' })} }),
    now: () => clock, sleep: async (ms: number) => { clock += ms }
  }
  return { deps, ready: () => { installed = true; ready = true }, installed: () => { installed = true }, runtime: createDockerSetup(deps) }
}
describe('bounded setup state machine', () => {
  it('does no work or confirmation when ready', async () => {
    const f = fixture(); f.ready()
    expect(await f.runtime.setup()).toEqual({ state: 'ready' })
    expect(f.deps.run).not.toHaveBeenCalled(); expect(f.deps.confirm).not.toHaveBeenCalled()
  })
  it('cancellation runs no command and retains a retryable state', async () => {
    const f = fixture(); f.deps.confirm.mockResolvedValue(false)
    expect((await f.runtime.setup()).state).toBe('not-installed')
    expect(f.deps.run).not.toHaveBeenCalled()
  })
  it.each(['darwin', 'linux'])('installs on %s and joins overlapping clicks', async platform => {
    const f = fixture(platform)
    const first = f.runtime.setup(), second = f.runtime.setup()
    expect(first).toBe(second)
    expect((await first).state).toBe('ready')
    expect(f.deps.confirm).toHaveBeenCalledTimes(1)
    expect(f.deps.run).toHaveBeenCalled()
  })
  it('starts an installed engine instead of reinstalling it', async () => {
    const f = fixture(); f.installed()
    expect((await f.runtime.status()).state).toBe('installed-not-running')
    expect((await f.runtime.setup()).state).toBe('ready')
    expect(f.deps.run).toHaveBeenCalledWith('/usr/bin/open', ['-a', 'Docker'], 120000)
  })
  it('Windows prerequisite setup ends in an explicit restart state', async () => {
    const f = fixture('win32')
    f.deps.run.mockImplementation(async program => { if (program === 'wsl.exe') {throw new Error('missing');} return '' })
    expect((await f.runtime.setup()).message).toContain('restart Windows')
  })
  it('installer failure and startup timeout cannot leave starting forever', async () => {
    const f = fixture(); f.deps.run.mockRejectedValue(new Error('Install cancelled'))
    expect(await f.runtime.setup()).toEqual({ state: 'error', message: 'Install cancelled' })
    f.deps.run.mockResolvedValue(''); f.installed()
    expect((await f.runtime.setup()).state).toBe('error')
    expect((await f.runtime.setup()).message).toContain('did not become ready')
  })
  it('reports permission denial as an actionable error', async () => {
    const f = fixture('linux'); f.deps.probe.mockRejectedValue(new Error('permission denied'))
    expect((await f.runtime.status()).message).toContain('access is denied')
  })
})
