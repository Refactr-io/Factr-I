import { act, cleanup } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { type MessageStreamHarness, renderMessageStream } from './test-harness'
import { STREAM_DELTA_FLUSH_MS } from './utils'

const SID = 'replay-session'

let stream: MessageStreamHarness

const texts = () =>
  (stream.state()?.messages ?? [])
    .flatMap(message => message.parts)
    .flatMap(part => (part.type === 'text' ? [part.text] : []))
    .join('|')

describe('a replay that lost frames while a reply is still streaming', () => {
  afterEach(() => {
    cleanup()
    vi.useRealTimers()
  })

  it('withholds the gapped partial and shows the whole reply when message.complete arrives', async () => {
    vi.useFakeTimers()
    stream = renderMessageStream(SID)
    await act(async () => {
      await Promise.resolve()
    })

    act(() => stream.handleEvent({ payload: { text: 'first part ' }, session_id: SID, type: 'message.delta' }))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(STREAM_DELTA_FLUSH_MS * 2)
    })
    expect(texts()).toBe('first part ')

    act(() => stream.handleEvent({ payload: {}, session_id: SID, type: 'session.replay_truncated' } as never))
    expect(texts()).toBe('')

    // Text after the hole is not shown either: it would read as a reply missing its middle.
    act(() => stream.handleEvent({ payload: { text: 'tail after the hole' }, session_id: SID, type: 'message.delta' }))
    await act(async () => {
      await vi.advanceTimersByTimeAsync(STREAM_DELTA_FLUSH_MS * 2)
    })
    expect(texts()).toBe('')

    act(() =>
      stream.handleEvent({
        payload: { status: 'complete', text: 'first part middle tail after the hole' },
        session_id: SID,
        type: 'message.complete'
      })
    )
    await act(async () => {
      await vi.advanceTimersByTimeAsync(STREAM_DELTA_FLUSH_MS * 2)
    })
    expect(texts()).toBe('first part middle tail after the hole')
  })
})
