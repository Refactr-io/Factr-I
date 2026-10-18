/**
 * A cold open with a full pool queues behind an eviction and the engine's own start, so the first ask can time out
 * while the engine it needs is still coming up. One more ask after that settles is what keeps the click from ending in
 * "Could not open <bot>'s chat"; an engine that genuinely cannot answer (older build) is still `undefined` at once.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { requestForBot } = vi.hoisted(() => ({ requestForBot: vi.fn() }))

vi.mock('./routing', () => ({
  backendTargetProfile: (_route: unknown, name: string) => name,
  botConnectionRoute: () => null,
  requestForBot
}))

import { engineBotChat, OPEN_RETRY_MS } from './engine-bot-chat'

const CHAT = { id: 'c1', resolved_id: 'c1', root_title: 'Bot Chat', title: 'Bot Chat' }

beforeEach(() => {
  vi.useFakeTimers()
  requestForBot.mockReset()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('engineBotChat on a cold open', () => {
  it('asks once more after a slot wait or descriptor timeout, and returns the second answer', async () => {
    for (const message of [
      'Timed out resolving the connection descriptor for profile "epsilon"',
      'Local backend start for "epsilon" is waiting for a free local slot (3/3 busy, 1 queued)',
      'Backend for "epsilon" was retired; open it explicitly to reconnect.'
    ]) {
      requestForBot.mockReset()
      requestForBot.mockRejectedValueOnce(new Error(message)).mockResolvedValueOnce({ canonical_session: CHAT })

      const pending = engineBotChat({ name: 'epsilon' }, { spawnPriority: 'foreground' })

      await vi.advanceTimersByTimeAsync(OPEN_RETRY_MS + 10)
      expect(await pending).toEqual(CHAT)
      expect(requestForBot).toHaveBeenCalledTimes(2)
    }
  })

  it('retries exactly once: a second failure is "cannot answer", not a loop', async () => {
    requestForBot.mockRejectedValue(new Error('timed out'))

    const pending = engineBotChat({ name: 'epsilon' })

    await vi.advanceTimersByTimeAsync(OPEN_RETRY_MS * 3)
    expect(await pending).toBeUndefined()
    expect(requestForBot).toHaveBeenCalledTimes(2)
  })

  it('does not wait for an engine that has no registry at all (an older build)', async () => {
    requestForBot.mockRejectedValue(new Error('unknown method bot.chat.get'))

    expect(await engineBotChat({ name: 'epsilon' })).toBeUndefined()
    expect(requestForBot).toHaveBeenCalledTimes(1)
  })

  it('answers at once when the engine answers, including "no chat yet"', async () => {
    requestForBot.mockResolvedValueOnce({ canonical_session: null })
    expect(await engineBotChat({ name: 'a' })).toBeNull()
    requestForBot.mockResolvedValueOnce({ canonical_session: CHAT })
    expect(await engineBotChat({ name: 'a' })).toEqual(CHAT)
    expect(requestForBot).toHaveBeenCalledTimes(2)
  })
})
