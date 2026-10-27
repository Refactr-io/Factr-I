import { beforeEach, describe, expect, it, vi } from 'vitest'

import * as system from '@/api/system'
import { $notifications } from '@/store/notifications'

import { ensureSttReady } from './stt-prepare'

vi.mock('@/api/system', () => ({ getSttStatus: vi.fn(), prepareStt: vi.fn() }))

const status = (state: system.SttPrepareStatus['state'], detail = '') => ({ detail, state })

describe('ensureSttReady', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    $notifications.set([])
    vi.mocked(system.getSttStatus).mockReset()
    vi.mocked(system.prepareStt).mockReset()
  })

  it('does nothing when dictation needs nothing (cloud provider, or already ready)', async () => {
    vi.mocked(system.getSttStatus).mockResolvedValue(status('not_needed'))
    expect((await ensureSttReady()).state).toBe('not_needed')
    expect(system.prepareStt).not.toHaveBeenCalled()
    expect($notifications.get()).toHaveLength(0)
  })

  it('starts the preparation on first use, follows it, and says when it is ready', async () => {
    vi.mocked(system.getSttStatus)
      .mockResolvedValueOnce(status('idle'))
      .mockResolvedValueOnce(status('downloading'))
      .mockResolvedValueOnce(status('ready'))
    vi.mocked(system.prepareStt).mockResolvedValue(status('installing'))

    const done = ensureSttReady()

    await vi.advanceTimersByTimeAsync(5000)
    expect((await done).state).toBe('ready')
    expect(system.prepareStt).toHaveBeenCalledTimes(1)
    expect($notifications.get()[0]?.kind).toBe('success')
  })

  it('shows why it failed, with a way to try again', async () => {
    vi.mocked(system.getSttStatus).mockResolvedValueOnce(status('idle'))
    vi.mocked(system.prepareStt).mockResolvedValue(status('failed', 'offline'))

    expect((await ensureSttReady()).state).toBe('failed')

    const notice = $notifications.get()[0]

    expect(notice?.kind).toBe('error')
    expect(notice?.detail).toBe('offline')
    expect(notice?.action?.label).toBeTruthy()
  })

  it('shares one preparation between presses and survives an older backend', async () => {
    vi.mocked(system.getSttStatus).mockRejectedValue(new Error('404'))

    const [a, b] = [ensureSttReady(), ensureSttReady()]

    expect(await a).toEqual(await b)
    expect((await a).state).toBe('not_needed')
    expect(system.getSttStatus).toHaveBeenCalledTimes(1)
  })
})
