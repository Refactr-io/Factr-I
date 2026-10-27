import { describe, expect, it } from 'vitest'

import { notSentReason } from './not-sent'

describe('a send the engine connection cannot take', () => {
  it('says the engine is still starting while it connects', () => {
    expect(notSentReason('idle')).toBe('starting')
    expect(notSentReason('connecting')).toBe('starting')
  })

  it('says the engine is not running when the connection is closed or failed', () => {
    expect(notSentReason('closed')).toBe('stopped')
    expect(notSentReason('error')).toBe('stopped')
  })

  it('stays quiet when the connection is open (a read-only run explains itself)', () => {
    expect(notSentReason('open')).toBeNull()
  })
})
