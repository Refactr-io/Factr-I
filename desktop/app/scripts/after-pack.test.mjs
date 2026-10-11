import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { test } from 'vitest'

import afterPack, { assertBundleVariant, fuseOptionsFor } from './after-pack.mjs'

// A fake packed app whose dist/ carries the given build-variant marker (or none).
function packedApp(marker) {
  const appOutDir = fs.mkdtempSync(path.join(os.tmpdir(), 'after-pack-'))
  const resources =
    process.platform === 'darwin' ? path.join(appOutDir, 'Factr-I.app', 'Contents', 'Resources') : path.join(appOutDir, 'resources')
  const dist = path.join(resources, 'app.asar.unpacked', 'dist')
  fs.mkdirSync(dist, { recursive: true })
  if (marker !== undefined) fs.writeFileSync(path.join(dist, 'build-variant.json'), JSON.stringify(marker))
  return { appOutDir, packager: { appInfo: { productFilename: 'Factr-I' } } }
}

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
  const context = packedApp({ qa: false })
  context.packager = {
    appInfo: { productFilename: 'Factr-I' },
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

test('the bundle variant must match the fuse variant: a stale QA or release dist/ fails the build', () => {
  assert.doesNotThrow(() => assertBundleVariant(packedApp({ qa: false }), {}))
  assert.doesNotThrow(() => assertBundleVariant(packedApp({ qa: true }), { FACTR_QA_FUSES: '1' }))
  assert.throws(() => assertBundleVariant(packedApp({ qa: true }), {}), /QA variant but this build is release/)
  assert.throws(() => assertBundleVariant(packedApp({ qa: false }), { FACTR_QA_FUSES: '1' }), /release variant but this build is QA/)
  assert.throws(() => assertBundleVariant(packedApp(undefined), {}), /cannot read the bundle variant/)
})
