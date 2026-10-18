import { describe, expect, it } from 'vitest'
import { isLegacyDelegatedRoutine, routineInputError, routinePrompt } from './cron'

describe('routine owner engine execution', () => {
  it('passes literal instructions for every owner without CLI delegation', () => {
    const instruction = "Line one $(literal) `literal`\nLine two"
    for (const owner of ['default', 'research']) {
      expect(routinePrompt(owner, 'Digest', instruction, 'default')).toBe(instruction)
    }
  })
  it('keeps unsafe pre-hardening jobs identifiable for the existing pause policy', () => {
    expect(isLegacyDelegatedRoutine({ job_id: 'x', name: '[bot:research] Audit', prompt: 'You are running the scheduled routine \"Audit\"' })).toBe(true)
    expect(isLegacyDelegatedRoutine({ job_id: 'x', name: 'Audit', prompt: 'Inspect' })).toBe(false)
  })
})

describe('input the gateway cannot store', () => {
  it('rejects NUL before cron creation, naming the offending field', () => {
    expect(routineInputError('', 'Instruction')).toBe('Give this routine a name.')
    expect(routineInputError('Normal title', 'Normal instruction')).toBeNull()
    expect(routineInputError('Bad\0title', 'Normal instruction')).toMatch(/NUL.*U\+0000/)
    expect(routineInputError('Normal title', 'Bad\0instruction')).toMatch(/NUL.*U\+0000/)
  })
})
