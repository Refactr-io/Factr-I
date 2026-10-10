import assert from 'node:assert/strict'
import { test } from 'vitest'

import afterPack, { fuseOptionsFor } from './after-pack.mjs'

test('a release build turns the three fuses off; FACTR_QA_FUSES=1 keeps them on', () => {
  const off = { runAsNode: false, enableNodeOptionsEnvironmentVariable: false, enableNodeCliInspectArguments: false }
  const on = { runAsNode: true, enableNodeOptionsEnvironmentVariable: true, enableNodeCliInspectArguments: true }
  assert.deepEqual(fuseOptionsFor({}, 'darwin', 'arm64'), { ...off, resetAdHocDarwinSignature: true })
  assert.deepEqual(fuseOptionsFor({ FACTR_QA_FUSES: '0' }, 'linux', 'x64'), {
    ...off,
    resetAdHocDarwinSignature: false
  })
  assert.deepEqual(fuseOptionsFor({ FACTR_QA_FUSES: '1' }, 'darwin', 'arm64'), {
    ...on,
    resetAdHocDarwinSignature: true
  })
  assert.equal(fuseOptionsFor({}, 'darwin', 'x64').resetAdHocDarwinSignature, false)
  assert.equal('onlyLoadAppFromAsar' in fuseOptionsFor({}), false, 'OnlyLoadAppFromAsar is left as Electron ships it')
})

test('the hook flips the fuses through electron-builder before signing', async () => {
  const calls = []
  const context = { appOutDir: 'x' }
  context.packager = {
    generateFuseConfig: async options => ({ generated: options }),
    addElectronFuses: async (ctx, config) => calls.push([ctx, config])
  }
  const before = process.env.FACTR_QA_FUSES
  delete process.env.FACTR_QA_FUSES
  try {
    await afterPack(context)
  } finally {
    if (before !== undefined) process.env.FACTR_QA_FUSES = before
  }
  assert.equal(calls.length, 1)
  assert.equal(calls[0][0], context)
  assert.equal(calls[0][1].generated.runAsNode, false)
})
