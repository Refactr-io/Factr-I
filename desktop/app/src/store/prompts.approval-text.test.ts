import { describe, expect, it } from 'vitest'

import { approvalCommandText } from './prompts'

describe('approvalCommandText', () => {
  it('prefers the whole command when the engine sent one beside the shortened text', () => {
    expect(approvalCommandText({ command: 'echo a…', command_full: 'echo a very long command' })).toBe(
      'echo a very long command'
    )
  })

  it('falls back to the shown command, and to empty for anything else', () => {
    expect(approvalCommandText({ command: 'ls' })).toBe('ls')
    expect(approvalCommandText({ command: 5, command_full: '' })).toBe('')
    expect(approvalCommandText({})).toBe('')
  })
})
