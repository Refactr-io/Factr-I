import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'

import { test } from 'vitest'

import { engineTokenLaunch } from './engine-token'

test('the engine gets its token on stdin, never in env or argv', async () => {
  const launch = engineTokenLaunch('factr', 'tok-abc')
  assert.deepEqual(launch.args, ['--token-stdin'])
  assert.equal(launch.env.FACTR_DASHBOARD_SESSION_TOKEN, undefined)
  assert.ok(!JSON.stringify(launch.args).includes('tok-abc'))
  assert.equal(launch.stdio[0], 'pipe')

  const child = spawn('cat', [], { stdio: launch.stdio })
  let out = ''
  child.stdout!.on('data', d => (out += d))
  launch.deliver(child)
  await new Promise(resolve => child.once('close', resolve))
  assert.equal(out, 'tok-abc\n')
})

test('python backends keep the env var', () => {
  const launch = engineTokenLaunch('python', 'tok-abc')
  assert.equal(launch.env.FACTR_DASHBOARD_SESSION_TOKEN, 'tok-abc')
  assert.deepEqual(launch.args, [])
  assert.equal(launch.stdio[0], 'ignore')
})
