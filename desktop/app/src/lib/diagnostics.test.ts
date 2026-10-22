import { describe, expect, it } from 'vitest'

import { buildDiagnostics, redactText, redactValue } from './diagnostics'

// Every secret shape the report must never contain, built so no real credential is committed.
const SECRETS = [
  'sk-' + 'a1B2c3D4e5F6g7H8',
  'ghp_' + 'A'.repeat(36),
  'github_pat_' + 'B'.repeat(30),
  'xoxb-' + '1234567890-abcdef',
  'AKIA' + 'IOSFODNN7EXAMPLE',
  'AIza' + 'C'.repeat(35),
  ['eyJhbGciOiJIUzI1NiJ9', 'eyJzdWIiOiIxMjM0NTY3ODkwIn0', 'dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U'].join('.'),
  'f'.repeat(48),
  'hunter2-super-secret',
  's3cr3tP@ss'
]

describe('redaction', () => {
  it('removes every secret shape from free text', () => {
    const text = SECRETS.slice(0, 8).join(' | ')
    const out = redactText(text)

    for (const secret of SECRETS.slice(0, 8)) {
      expect(out).not.toContain(secret)
    }
  })

  it('strips credentials and secret query values from URLs and keeps the rest', () => {
    const out = redactText('https://user:s3cr3tP@ss@example.com/v1?key=abc123&model=x&token=zzz')

    expect(out).not.toContain('s3cr3tP@ss')
    expect(out).not.toContain('abc123')
    expect(out).not.toContain('zzz')
    expect(out).toContain('example.com/v1')
    expect(out).toContain('model=x')
  })

  it('folds any user home directory to ~', () => {
    expect(redactText('/Users/anyone/a /home/other/b C:\\Users\\third\\c')).toBe('~/a ~/b ~\\c')
  })

  it('folds the home directory to ~', () => {
    expect(redactText('/Users/someone/project/file.txt', '/Users/someone')).toBe('~/project/file.txt')
  })

  it('hides secret-named values at any depth, lists included, and leaves ordinary settings', () => {
    const config = {
      budget: { max_tokens: 0 },
      model: { api_key: SECRETS[0], default: 'gpt-x', base_url: 'https://user:hunter2-super-secret@host.test/v1' },
      providers: [{ name: 'a', token: SECRETS[1] }],
      mcp: {
        servers: {
          x: { env: { GITHUB_TOKEN: SECRETS[1], PATH: '/usr/bin' }, headers: { Authorization: 'Bearer ' + SECRETS[2] } }
        }
      },
      tts: { elevenlabs: { api_keys: [SECRETS[3]] } },
      pin: 1234,
      custom: { env: { MY_SERVICE_VALUE: 'plain-looking-secret-value' } },
      terminal: { reduce_output: true }
    }

    const out = JSON.stringify(redactValue(config))

    for (const secret of SECRETS) {
      expect(out).not.toContain(secret)
    }

    expect(out).not.toContain('plain-looking-secret-value')
    expect(out).toContain('MY_SERVICE_VALUE')
    expect(out).toContain('gpt-x')
    expect(out).toContain('"reduce_output":true')
    expect(out).toContain('"max_tokens":0')
    expect(out).not.toContain('1234')
  })
})

describe('buildDiagnostics', () => {
  it('carries the facts a support request needs and none of the secrets', () => {
    const report = buildDiagnostics({
      appVersion: '0.0.11',
      config: { model: { api_key: SECRETS[0] }, approvals: { mode: 'smart' } },
      home: '/Users/someone',
      lastError: `401 from https://api.test/v1?api_key=${SECRETS[8]} for ${SECRETS[1]} in /Users/someone/work`,
      model: 'gpt-x',
      os: 'macOS 15',
      provider: 'openai'
    })

    expect(report).toContain('App version: 0.0.11')
    expect(report).toContain('Provider: openai')
    expect(report).toContain('Model: gpt-x')
    expect(report).toContain('"mode": "smart"')
    expect(report).toContain('~/work')

    for (const secret of SECRETS) {
      expect(report).not.toContain(secret)
    }
  })

  it('says unknown and none instead of failing on missing data', () => {
    const report = buildDiagnostics({})

    expect(report).toContain('App version: unknown')
    expect(report).toContain('Last error: none')
  })

  it('cuts a huge config', () => {
    const report = buildDiagnostics({ config: { blob: 'x'.repeat(100_000) } })

    expect(report.length).toBeLessThan(21_000)
    expect(report).toContain('(cut)')
  })
})
