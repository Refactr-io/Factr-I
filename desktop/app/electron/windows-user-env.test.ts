import assert from 'node:assert/strict'

import { test } from 'vitest'

import { expandWindowsEnvRefs, parseRegQueryValue, readWindowsUserEnvVar } from './windows-user-env'

// ── parseRegQueryValue ─────────────────────────────────────────────────────

test('parseRegQueryValue extracts a REG_SZ value', () => {
  const out = ['', 'HKEY_CURRENT_USER\\Environment', '    FACTR_CONFIG_HOME    REG_SZ    F:\\Factr\\data', ''].join(
    '\r\n'
  )
  assert.equal(parseRegQueryValue(out, 'FACTR_CONFIG_HOME'), 'F:\\Factr\\data')
})

test('parseRegQueryValue matches the name case-insensitively', () => {
  const out = 'HKEY_CURRENT_USER\\Environment\r\n    Factr_Config_Home    REG_EXPAND_SZ    %USERPROFILE%\\h\r\n'
  assert.equal(parseRegQueryValue(out, 'FACTR_CONFIG_HOME'), '%USERPROFILE%\\h')
})

test('parseRegQueryValue preserves spaces inside the value', () => {
  const out = '    FACTR_CONFIG_HOME    REG_SZ    C:\\Program Files\\Factr\r\n'
  assert.equal(parseRegQueryValue(out, 'FACTR_CONFIG_HOME'), 'C:\\Program Files\\Factr')
})

test('parseRegQueryValue returns null when the value line is absent', () => {
  const out = 'HKEY_CURRENT_USER\\Environment\r\n    Path    REG_SZ    C:\\x\r\n'
  assert.equal(parseRegQueryValue(out, 'FACTR_CONFIG_HOME'), null)
  assert.equal(parseRegQueryValue('', 'FACTR_CONFIG_HOME'), null)
  assert.equal(parseRegQueryValue('garbage', 'FACTR_CONFIG_HOME'), null)
})

// ── expandWindowsEnvRefs ───────────────────────────────────────────────────

test('expandWindowsEnvRefs expands %VAR% case-insensitively', () => {
  assert.equal(expandWindowsEnvRefs('%UserProfile%\\h', { USERPROFILE: 'C:\\Users\\jeff' }), 'C:\\Users\\jeff\\h')
})

test('expandWindowsEnvRefs leaves literal paths and unknown refs intact', () => {
  assert.equal(expandWindowsEnvRefs('F:\\Factr\\data', {}), 'F:\\Factr\\data')
  assert.equal(expandWindowsEnvRefs('%NOPE%\\x', {}), '%NOPE%\\x')
})

// ── readWindowsUserEnvVar ──────────────────────────────────────────────────

test('readWindowsUserEnvVar returns null off Windows without spawning', () => {
  let spawned = false

  const exec = () => {
    spawned = true

    return ''
  }

  assert.equal(readWindowsUserEnvVar('FACTR_CONFIG_HOME', { platform: 'linux', exec }), null)
  assert.equal(spawned, false)
})

test('readWindowsUserEnvVar queries HKCU\\Environment and expands the value', () => {
  const calls = []

  const exec = (cmd, args) => {
    calls.push([cmd, args])

    return 'HKEY_CURRENT_USER\\Environment\r\n    FACTR_CONFIG_HOME    REG_EXPAND_SZ    %DRIVE%\\Factr\r\n'
  }

  const value = readWindowsUserEnvVar('FACTR_CONFIG_HOME', {
    platform: 'win32',
    env: { DRIVE: 'F:' },
    exec
  })

  assert.equal(value, 'F:\\Factr')
  assert.deepEqual(calls, [['reg', ['query', 'HKCU\\Environment', '/v', 'FACTR_CONFIG_HOME']]])
})

test('readWindowsUserEnvVar returns null when reg exits non-zero (value missing)', () => {
  const exec = () => {
    throw new Error('reg exited 1')
  }

  assert.equal(readWindowsUserEnvVar('FACTR_CONFIG_HOME', { platform: 'win32', exec }), null)
})

test('readWindowsUserEnvVar returns null for an empty value', () => {
  const exec = () => '    FACTR_CONFIG_HOME    REG_SZ    \r\n'
  assert.equal(readWindowsUserEnvVar('FACTR_CONFIG_HOME', { platform: 'win32', exec }), null)
})
