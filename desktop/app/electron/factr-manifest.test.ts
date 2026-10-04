import { describe, expect, it } from 'vitest'

import { engineMismatch, parseFactrManifest } from './factr-manifest'

const manifest = parseFactrManifest(
  JSON.stringify({
    id: 'abc',
    engine: { version: '0.88.0', sha: '06ddb7d' },
    factr: { sha: 'f'.repeat(40) },
    python: { version: '3.12.12' }
  })
)

describe('factr bundle manifest', () => {
  it('parses only a usable manifest', () => {
    expect(manifest?.engine.sha).toBe('06ddb7d')
    expect(parseFactrManifest(null)).toBeNull()
    expect(parseFactrManifest('{}')).toBeNull()
    expect(parseFactrManifest('not json')).toBeNull()
  })

  it('accepts the engine it shipped with, whatever the short-sha length', () => {
    expect(engineMismatch(manifest, { engine: 'factr', version: '0.88.0', sha: '06ddb7d' })).toBeNull()
    expect(engineMismatch(manifest, { engine: 'factr', version: '0.88.0', sha: '06ddb7d6f' })).toBeNull()
    expect(engineMismatch(null, { engine: 'x' })).toBeNull()
  })

  it('refuses a stale engine, an engine that reports no build, and a foreign backend', () => {
    expect(engineMismatch(manifest, { engine: 'factr', version: '0.88.0', sha: '1234567' })).toMatch(/left over/)
    expect(engineMismatch(manifest, { engine: 'factr', version: '0.87.0', sha: '06ddb7d' })).toMatch(/does not match/)
    expect(engineMismatch(manifest, { engine: 'factr', version: '0.88.0' })).toMatch(/does not match/)
    expect(engineMismatch(manifest, { version: '0.17.6' })).toMatch(/Something other/)
  })

  it('checks the version alone when the packager could not probe the engine sha', () => {
    const unprobed = parseFactrManifest(JSON.stringify({ id: 'x', engine: { version: '0.88.0', sha: null } }))

    expect(engineMismatch(unprobed, { engine: 'factr', version: '0.88.0' })).toBeNull()
    expect(engineMismatch(unprobed, { engine: 'factr', version: '0.90.0' })).toMatch(/does not match/)
  })
})
