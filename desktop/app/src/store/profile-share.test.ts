import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { ProfileDesktopOverlay } from '@/types/factr'

vi.mock('@/store/gateway', async () => {
  const { atom } = await import('nanostores')

  return {
    $gateway: atom<unknown>(null),
    ensureGatewayForProfile: vi.fn(async () => undefined),
    openGatewayForProfile: vi.fn(async () => undefined)
  }
})
vi.mock('@/factr', () => ({
  exportProfileArchive: vi.fn(async () => ({ archive: '/tmp/out.tar.gz', ok: true })),
  getProfiles: vi.fn(async () => ({ profiles: [] })),
  importProfileArchive: vi.fn(async () => ({ desktop: null, name: 'imported', ok: true, path: '/tmp/p' })),
  setApiRequestProfile: vi.fn()
}))
vi.mock('@/lib/query-client', () => ({ invalidateProfileScopedQueries: vi.fn() }))
vi.mock('@/store/starmap', () => ({ resetStarmapGraph: vi.fn() }))

const { applyDesktopOverlay, buildDesktopOverlay, exportProfileBundle } = await import('./profile-share')
const { modePref } = await import('@/themes/context')
const { exportProfileArchive } = await import('@/factr')

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
})

describe('profile appearance overlay', () => {
  it('exports the mode without old skin or theme definitions', () => {
    modePref.assign('glam', 'dark')
    const overlay = buildDesktopOverlay('glam')
    expect(overlay.mode).toBe('dark')
    expect(Object.hasOwn(overlay, 'skin')).toBe(false)
    expect(Object.hasOwn(overlay, 'themes')).toBe(false)
  })

  it('ignores imported skins and accepts only light or dark', () => {
    applyDesktopOverlay('copy', { mode: 'dark', skin: 'catppuccin', themes: { x: {} } } as ProfileDesktopOverlay)
    expect(modePref.resolve('copy')).toBe('dark')
    applyDesktopOverlay('copy', { mode: 'system' })
    expect(modePref.resolve('copy')).toBe('dark')
  })

  it('stages the fixed appearance in desktop.json', async () => {
    modePref.assign('glam', 'light')
    await exportProfileBundle('glam', '/tmp/glam.tar.gz')
    const call = vi.mocked(exportProfileArchive).mock.calls[0]
    const overlay = JSON.parse(call[1]?.extraFiles?.['desktop.json'] ?? '{}')
    expect(overlay.mode).toBe('light')
    expect(overlay.skin).toBeUndefined()
  })
})
