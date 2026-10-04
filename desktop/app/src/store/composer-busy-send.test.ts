import { beforeEach, describe, expect, it, vi } from 'vitest'

const loadStore = async () => {
  vi.resetModules()

  return import('./composer-busy-send')
}

describe('busy-send preference', () => {
  beforeEach(() => window.localStorage.clear())

  it('defaults to queue and survives a restart in either state', async () => {
    const first = await loadStore()

    expect(first.$busySendMode.get()).toBe('queue')

    first.setBusySendMode('steer')
    expect((await loadStore()).$busySendMode.get()).toBe('steer')

    ;(await loadStore()).setBusySendMode('queue')
    expect((await loadStore()).$busySendMode.get()).toBe('queue')
  })
})
