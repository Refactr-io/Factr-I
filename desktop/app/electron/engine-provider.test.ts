import fs from 'node:fs'
import path from 'node:path'

import { describe, expect, it } from 'vitest'

import { ENGINE_PROVIDER_FLAG_VALUES, engineProviderArgs, isEngineProviderFlagValue } from './engine-provider'

/** The `--provider` values clap accepts: ProviderChoice's kebab-case names (or `#[value(name)]`) and aliases. */
function engineFlagValuesFromSource(): string[] {
  const source = fs.readFileSync(path.resolve(__dirname, '../../../engine/src/factr_runtime.rs'), 'utf8')
  const body = source.match(/pub enum ProviderChoice \{([\s\S]*?)\n\}/)?.[1] ?? ''
  const ids: string[] = []

  for (const m of body.replace(/\/\/\/[^\n]*/g, '').matchAll(/(#\[value\(([\s\S]*?)\)\]\s*)?([A-Z][A-Za-z0-9]*),/g)) {
    const attrs = m[2] ?? ''
    const name = attrs.match(/name\s*=\s*"([^"]+)"/)?.[1]
    ids.push(name ?? m[3].replace(/(?<=[a-z0-9])(?=[A-Z])/g, '-').toLowerCase())
    ids.push(...[...attrs.matchAll(/alias\s*=\s*"([^"]+)"/g)].map(a => a[1]))
  }

  return [...new Set(ids)].sort()
}

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
    expect(engineProviderArgs({ FACTR_PROVIDER: 'OpenAI-Codex' })).toEqual(['--provider', 'OpenAI-Codex'])
    expect(engineProviderArgs({ FACTR_MODEL: 'gpt-6-luna' })).toEqual(['--model', 'gpt-6-luna'])
    expect(engineProviderArgs({ FACTR_MODEL: 'qwen3:8b' })).toEqual(['--model', 'qwen3:8b'])
  })

  it('never turns a value the engine flag rejects into a flag: the engine reads the env var and explains', () => {
    // clap would exit with "invalid variant: definitely-bogus" before the port announcement.
    expect(engineProviderArgs({ FACTR_PROVIDER: 'definitely-bogus' })).toEqual([])
    expect(engineProviderArgs({ FACTR_PROVIDER: 'definitely-bogus', FACTR_MODEL: 'gpt-6-luna' })).toEqual([
      '--model',
      'gpt-6-luna'
    ])
    expect(engineProviderArgs({ FACTR_PROVIDER: '--allow-remote' })).toEqual([])
    expect(engineProviderArgs({ FACTR_MODEL: '--allow-remote' })).toEqual([])
    expect(engineProviderArgs({ FACTR_MODEL: 'two words' })).toEqual([])
  })

  it('accepts exactly the values the engine flag accepts', () => {
    expect(isEngineProviderFlagValue(' Claude ')).toBe(true)
    expect(isEngineProviderFlagValue('qwen-oauth')).toBe(false)
    expect([...ENGINE_PROVIDER_FLAG_VALUES].sort()).toEqual(engineFlagValuesFromSource())
  })
})
