import assert from 'node:assert/strict'
import { EventEmitter } from 'node:events'

import { test, vi } from 'vitest'

import { waitForBackendExit } from './backend-child'

test('backend exit escalation remains bounded when no exit or close arrives', async () => {
  vi.useFakeTimers()

  try {
    const child = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null, kill: vi.fn() })
    const waiting = waitForBackendExit(child, { forceKillProcessTree: () => {} }, 20)

    const outcome = waiting.then(
      () => null,
      error => error
    )

    await vi.advanceTimersByTimeAsync(1020)
    assert.match(String(await outcome), /did not exit/)
    assert.equal(child.exitCode, null)
    assert.equal(child.signalCode, null)
    assert.equal(child.kill.mock.calls.length, 1)
    assert.equal(child.kill.mock.calls[0][0], 'SIGKILL')
    assert.equal(child.listenerCount('exit'), 0)
    assert.equal(vi.getTimerCount(), 0)
  } finally {
    vi.useRealTimers()
  }
})

test('a failed escalation is not evidence that the child exited', async () => {
  vi.useFakeTimers()

  try {
    const child = Object.assign(new EventEmitter(), {
      exitCode: null,
      signalCode: null,
      kill: () => {
        throw new Error('signal denied')
      }
    })

    const outcome = waitForBackendExit(child, { forceKillProcessTree: () => {} }, 20).then(
      () => null,
      error => error
    )

    await vi.advanceTimersByTimeAsync(1020)
    assert.match(String(await outcome), /did not exit/)
    assert.equal(child.listenerCount('exit'), 0)
    assert.equal(vi.getTimerCount(), 0)
  } finally {
    vi.useRealTimers()
  }
})

test('whatever the backend left running is reaped once it has exited', async () => {
  const signals: Array<[number, number | string]> = []
  const child = Object.assign(new EventEmitter(), { exitCode: null as number | null, signalCode: null, kill: vi.fn(), pid: 4242 })
  let groupAlive = true

  const killGroup = (pgid: number, signal: number | string) => {
    signals.push([pgid, signal])

    if (!groupAlive) {
      throw new Error('ESRCH')
    }
  }

  const waiting = waitForBackendExit(child, { forceKillProcessTree: () => {}, isWindows: false, killGroup }, 1000)

  child.exitCode = 0
  child.emit('exit')
  await waiting

  // Probed with signal 0, then killed: the group still had a member (an installer shutting down).
  assert.deepEqual(signals, [
    [-4242, 0],
    [-4242, 'SIGKILL']
  ])

  signals.length = 0
  groupAlive = false
  const second = Object.assign(new EventEmitter(), { exitCode: null as number | null, signalCode: null, kill: vi.fn(), pid: 4243 })
  const done = waitForBackendExit(second, { forceKillProcessTree: () => {}, isWindows: false, killGroup }, 1000)

  second.exitCode = 0
  second.emit('exit')
  await done
  assert.deepEqual(signals, [[-4243, 0]], 'an empty group is left alone')
})
