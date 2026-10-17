import { describe, expect, it, vi } from 'vitest'

vi.mock('@factr/plugin-sdk', () => ({
  host: {},
  resolveSiblingWsUrl: vi.fn(async () => 'ws://127.0.0.1:5555/api/display/ws?token=ENGINE&ticket=ONESHOT')
}))

vi.mock('./routing', () => ({ resolveBotConnectionRoute: () => ({ route: null, status: 'resolved' }) }))

import { resolveScreenWsUrl } from './screen-connection'

describe('resolveScreenWsUrl', () => {
  it('keeps the engine token, drops the one-shot gateway ticket, asks for the feature runtime, adds the display ticket', async () => {
    const url = new URL(await resolveScreenWsUrl({ name: 'scout' } as never, 'DISPLAY'))

    expect(url.pathname).toBe('/api/display/ws')
    expect(url.searchParams.get('token')).toBe('ENGINE')
    expect(url.searchParams.get('ticket')).toBeNull()
    expect(url.searchParams.get('feature')).toBe('1')
    expect(url.searchParams.get('display_ticket')).toBe('DISPLAY')
  })
})
