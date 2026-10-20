import { afterEach, beforeEach, expect, it, vi } from 'vitest'

const created: { freq: number }[] = []

const fakeContext = () => ({
  createGain: () => ({ connect: vi.fn(), gain: { exponentialRampToValueAtTime: vi.fn(), setValueAtTime: vi.fn(), value: 1 } }),
  createOscillator: () => {
    const osc = { connect: vi.fn(), frequency: { setValueAtTime: (f: number) => created.push({ freq: f }) }, start: vi.fn(), stop: vi.fn(), type: '' }

    return osc
  },
  currentTime: 0,
  destination: {},
  state: 'running'
})

beforeEach(() => {
  created.length = 0
  vi.resetModules()
  vi.stubGlobal(
    'AudioContext',
    class {
      constructor() {
        return fakeContext()
      }
    }
  )
})

afterEach(() => vi.unstubAllGlobals())

it('plays each cue as its own pings', async () => {
  const { playSfx } = await import('./sfx')
  playSfx('approval')
  expect(created.map(c => c.freq)).toEqual([659, 880])
  created.length = 0
  playSfx('exitVoice')
  expect(created).toHaveLength(3)
  created.length = 0
  playSfx('enterVoice')
  expect(created).toHaveLength(2)
  created.length = 0
  playSfx('disconnected')
  expect(created).toHaveLength(2)
})

it('is silent while sounds are muted', async () => {
  const { setHapticsMuted } = await import('@/store/haptics')
  const { playSfx } = await import('./sfx')
  setHapticsMuted(true)
  playSfx('approval')
  expect(created).toHaveLength(0)
})
