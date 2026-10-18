import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { registerHapticTrigger } from '@/lib/haptics'
import { $hapticsMuted } from '@/store/haptics'

import { HapticsProvider } from './haptics-provider'

const { trigger } = vi.hoisted(() => ({ trigger: vi.fn() }))
vi.mock('web-haptics/react', () => ({ useWebHaptics: () => ({ trigger }) }))
vi.mock('@/lib/haptics', () => ({ registerHapticTrigger: vi.fn() }))

let idle: IdleRequestCallback
const close = vi.fn().mockResolvedValue(undefined)
const constructed = vi.fn()
const cancel = vi.fn()

const request = vi.fn((callback: IdleRequestCallback) => {
  idle = callback

  return 17
})

beforeEach(() => {
  vi.clearAllMocks()
  $hapticsMuted.set(false)
  vi.stubGlobal('requestIdleCallback', request)
  vi.stubGlobal('cancelIdleCallback', cancel)
  vi.stubGlobal(
    'AudioContext',
    class {
      close = close
      constructor() {
        constructed()
      }
    }
  )
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  $hapticsMuted.set(false)
})

describe('haptics audio warm-up', () => {
  it('does not start the audio service for a muted session', () => {
    $hapticsMuted.set(true)
    render(<HapticsProvider>Readable content</HapticsProvider>)

    expect(screen.getByText('Readable content')).toBeTruthy()
    expect(request).not.toHaveBeenCalled()
    expect(constructed).not.toHaveBeenCalled()
    expect(registerHapticTrigger).toHaveBeenLastCalledWith(null)
  })

  it('preserves the enabled idle timing and closes the throwaway context', async () => {
    render(<HapticsProvider>Content</HapticsProvider>)
    expect(request).toHaveBeenCalledWith(expect.any(Function), { timeout: 2000 })
    expect(registerHapticTrigger).toHaveBeenLastCalledWith(trigger)

    await act(async () => idle({ didTimeout: false, timeRemaining: () => 50 }))
    expect(constructed).toHaveBeenCalledOnce()
    expect(close).toHaveBeenCalledOnce()
  })

  it('cancels a queued warm-up on mute and guards an already-delivered callback', async () => {
    render(<HapticsProvider>Content</HapticsProvider>)
    act(() => $hapticsMuted.set(true))

    expect(cancel).toHaveBeenCalledWith(17)
    await act(async () => idle({ didTimeout: false, timeRemaining: () => 50 }))
    expect(constructed).not.toHaveBeenCalled()
  })

  it('warms after unmuting a session that started muted', async () => {
    $hapticsMuted.set(true)
    render(<HapticsProvider>Content</HapticsProvider>)
    act(() => $hapticsMuted.set(false))

    expect(request).toHaveBeenCalledOnce()
    await act(async () => idle({ didTimeout: false, timeRemaining: () => 50 }))
    expect(constructed).toHaveBeenCalledOnce()
  })

  it('cancels the pending idle callback on unmount', () => {
    const { unmount } = render(<HapticsProvider>Content</HapticsProvider>)
    unmount()

    expect(cancel).toHaveBeenCalledWith(17)
    expect(registerHapticTrigger).toHaveBeenLastCalledWith(null)
  })
})
