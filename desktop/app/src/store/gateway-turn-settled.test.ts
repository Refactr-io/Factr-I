import { describe, expect, it } from 'vitest'

import { isTurnSettledEvent } from './gateway'

describe('isTurnSettledEvent', () => {
  it('settles on the Python gateway edge and on the engine message.complete', () => {
    expect(isTurnSettledEvent({ type: 'session.info', payload: { running: false } } as never)).toBe(true)
    expect(isTurnSettledEvent({ type: 'message.complete', payload: {} } as never)).toBe(true)
  })

  it('does not settle while the turn runs or on unrelated frames', () => {
    expect(isTurnSettledEvent({ type: 'session.info', payload: { running: true } } as never)).toBe(false)
    expect(isTurnSettledEvent({ type: 'session.info', payload: {} } as never)).toBe(false)
    expect(isTurnSettledEvent({ type: 'message.delta', payload: {} } as never)).toBe(false)
  })
})
