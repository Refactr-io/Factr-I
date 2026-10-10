import { describe, expect, it } from 'vitest'

import { engineProviderArgs } from './engine-provider'

describe('engineProviderArgs', () => {
  it('passes nothing without an explicit choice, so the engine follows the saved or signed-in provider', () => {
    expect(engineProviderArgs({})).toEqual([])
    expect(engineProviderArgs({ FACTR_PROVIDER: '  ', FACTR_MODEL: '' })).toEqual([])
  })

  it('passes an explicit FACTR_PROVIDER / FACTR_MODEL through unchanged', () => {
    expect(engineProviderArgs({ FACTR_PROVIDER: 'openai', FACTR_MODEL: 'gpt-6-luna' })).toEqual([
      '--provider',
      'openai',
      '--model',
      'gpt-6-luna'
    ])
    expect(engineProviderArgs({ FACTR_PROVIDER: 'ollama' })).toEqual(['--provider', 'ollama'])
    expect(engineProviderArgs({ FACTR_MODEL: 'gpt-6-luna' })).toEqual(['--model', 'gpt-6-luna'])
  })
})
