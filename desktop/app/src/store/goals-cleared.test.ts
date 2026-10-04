import { beforeEach, describe, expect, it } from 'vitest'

import { $goalsBySession, applyGoalPayload, applyGoalStatusText } from './goals'

describe('goal indicator clearing', () => {
  beforeEach(() => $goalsBySession.set({}))

  it('clears on the engine sentence and on a structured cleared state', () => {
    applyGoalStatusText('s', '⊙ Goal set (budget 20 turns): ship it')
    expect($goalsBySession.get().s?.status).toBe('active')
    applyGoalStatusText('s', '✓ Goal cleared.')
    expect($goalsBySession.get().s).toBeUndefined()

    applyGoalStatusText('s', '⊙ Goal set (budget 20 turns): ship it')
    applyGoalPayload('s', { status: 'cleared' })
    expect($goalsBySession.get().s).toBeUndefined()
  })
})
