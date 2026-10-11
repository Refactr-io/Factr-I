import assert from 'node:assert/strict'

import { test } from 'vitest'

import {
  isLockedDownBuild,
  isQaBuild,
  isRemoteDebuggingSwitch,
  REMOTE_DEBUGGING_SWITCHES,
  stripRemoteDebuggingSwitches
} from './release-debug-lockdown'

const fakeCommandLine = (initial: string[]) => {
  const present = new Set(initial)

  return {
    present,
    hasSwitch: (name: string) => present.has(name),
    removeSwitch: (name: string) => void present.delete(name)
  }
}

test('only a packaged, non-QA build is locked down', () => {
  assert.equal(isLockedDownBuild(true, false), true)
  assert.equal(isLockedDownBuild(true, true), false, 'the QA variant keeps the debugging port')
  assert.equal(isLockedDownBuild(false, false), false, 'an unpackaged developer run is left alone')
  assert.equal(isLockedDownBuild(false, true), false)
})

test('every remote debugging switch is removed and reported; others stay', () => {
  const cl = fakeCommandLine([...REMOTE_DEBUGGING_SWITCHES, 'user-data-dir', 'disable-renderer-backgrounding'])

  assert.deepEqual(stripRemoteDebuggingSwitches(cl), [...REMOTE_DEBUGGING_SWITCHES])
  assert.deepEqual([...cl.present].sort(), ['disable-renderer-backgrounding', 'user-data-dir'])
})

test('nothing to strip is a quiet no-op', () => {
  const cl = fakeCommandLine(['user-data-dir'])

  assert.deepEqual(stripRemoteDebuggingSwitches(cl), [])
  assert.deepEqual([...cl.present], ['user-data-dir'])
})

test('config.yaml electron_flags names are recognised with or without dashes', () => {
  for (const name of ['remote-debugging-port', '--remote-debugging-pipe', ' remote-allow-origins ']) {
    assert.equal(isRemoteDebuggingSwitch(name), true, name)
  }

  for (const name of ['js-flags', 'disable-gpu', 'inspect', 'user-data-dir']) {
    assert.equal(isRemoteDebuggingSwitch(name), false, name)
  }
})

test('an unbundled run reads as not-QA, so the lockdown fails closed', () => {
  const before = process.env.FACTR_QA_BUILD
  delete process.env.FACTR_QA_BUILD

  try {
    assert.equal(isQaBuild(), false)
  } finally {
    if (before !== undefined) {process.env.FACTR_QA_BUILD = before}
  }
})
