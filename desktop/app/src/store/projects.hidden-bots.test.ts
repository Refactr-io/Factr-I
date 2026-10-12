import { atom } from 'nanostores'
import { describe, expect, it, vi } from 'vitest'

import { $activeGatewayProfile, setShowAllProfiles } from '@/store/profile'

import { projectProfile, refreshProjectTree } from './projects'

// With Bot Mode hidden the default profile is the only one: "all profiles" is that profile and its project tree
// loads over the gateway's projects.tree RPC (the engine does not serve the all-profiles REST fan-out).
vi.mock('@/lib/hidden-features', () => ({ botsHidden: true, HIDDEN_BUNDLED_PLUGINS: new Set(['factr-bots']) }))
vi.mock('@/i18n', () => ({ translateNow: (key: string) => key }))
vi.mock('@/store/notifications', () => ({ notify: vi.fn() }))
vi.mock('@/lib/desktop-fs', () => ({
  desktopDefaultCwd: vi.fn(),
  isDesktopFsRemoteMode: vi.fn(),
  selectDesktopPaths: vi.fn(),
  writeDesktopFileText: vi.fn()
}))
vi.mock('@/store/gateway', () => ({ $gateway: atom(null), activeGateway: vi.fn(), ensureActiveGatewayOpen: vi.fn() }))
vi.mock('@/factr', () => ({
  getFactrConfig: vi.fn(),
  getProfiles: vi.fn(),
  factrApi: vi.fn(),
  setApiRequestProfile: vi.fn(),
  STARTUP_REQUEST_TIMEOUT_MS: 1000
}))

const gw = await import('@/store/gateway')

describe('projects with Bot Mode hidden', () => {
  it('uses the default profile and the projects.tree RPC in the all-profiles view', async () => {
    const request = vi.fn().mockResolvedValue({ projects: [], sessions: [] })
    const gateway = { connectionState: 'open', request }
    vi.mocked(gw.activeGateway).mockReturnValue(gateway as never)
    gw.$gateway.set(gateway as never)
    $activeGatewayProfile.set('default')
    setShowAllProfiles(true)

    expect(projectProfile()).toBe('default')
    await refreshProjectTree()
    expect(request.mock.calls.some(([method]) => method === 'projects.tree')).toBe(true)
    setShowAllProfiles(false)
  })
})
