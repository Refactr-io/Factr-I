import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { registerHapticTrigger } from '@/lib/haptics'
import { $hapticsMuted } from '@/store/haptics'

import { HapticsProvider } from './haptics-provider'

const { trigger, useWebHaptics } = vi.hoisted(() => {
  const trigger = vi.fn()

  return { trigger, useWebHaptics: vi.fn(() => ({ trigger })) }
})

vi.mock('web-haptics/react', () => ({ useWebHaptics }))
vi.mock('@/lib/haptics', () => ({ registerHapticTrigger: vi.fn() }))

beforeEach(() => {
  vi.clearAllMocks()
  $hapticsMuted.set(false)
})

afterEach(() => {
  cleanup()
  $hapticsMuted.set(false)
})

it('registers the trigger, and never asks the library for its audible debug clicks', () => {
  render(
    <HapticsProvider>
      <span>child</span>
    </HapticsProvider>
  )

  expect(screen.getByText('child')).toBeTruthy()
  expect(useWebHaptics).toHaveBeenCalledWith(expect.objectContaining({ debug: false }))
  expect(registerHapticTrigger).toHaveBeenLastCalledWith(trigger)
})

it('drops the trigger while muted and takes it back on unmute', () => {
  render(<HapticsProvider>x</HapticsProvider>)
  act(() => $hapticsMuted.set(true))
  expect(registerHapticTrigger).toHaveBeenLastCalledWith(null)
  act(() => $hapticsMuted.set(false))
  expect(registerHapticTrigger).toHaveBeenLastCalledWith(trigger)
})
