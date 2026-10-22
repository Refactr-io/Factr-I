import assert from 'node:assert/strict'
import { test } from 'vitest'

import { RTK_ASSETS, publishedHash, sha256, verifyAsset } from './stage-rtk.mjs'

const bytes = Buffer.from('rtk')
const asset = 'rtk-oss-x.tar.gz'

test('an asset passes only when the pin and the published checksum both match its bytes', () => {
  const good = sha256(bytes)
  verifyAsset({ bytes, pinned: good, published: good, asset })
  assert.throws(() => verifyAsset({ bytes, pinned: 'a'.repeat(64), published: 'a'.repeat(64), asset }), /does not match the pinned/)
  assert.throws(() => verifyAsset({ bytes, pinned: good, published: 'b'.repeat(64), asset }), /checksums.txt lists/)
  assert.throws(() => verifyAsset({ bytes, pinned: good, published: null, asset }), /lists nothing/)
})

test('the published hash is read by exact asset name', () => {
  const sums = `${'1'.repeat(64)}  rtk-oss-x.tar.gz.sig\n${'2'.repeat(64)}  rtk-oss-x.tar.gz\n`
  assert.equal(publishedHash(sums, 'rtk-oss-x.tar.gz'), '2'.repeat(64))
  assert.equal(publishedHash(sums, 'missing'), null)
})

test('every shipped platform pins an Apache-only rtk-oss asset with a sha256', () => {
  assert.deepEqual(Object.keys(RTK_ASSETS).sort(), ['darwin-arm64', 'darwin-x64', 'linux-x64', 'win32-x64'])
  for (const { asset: name, sha256: hash } of Object.values(RTK_ASSETS)) {
    assert.match(name, /^rtk-oss-/)
    assert.match(hash, /^[0-9a-f]{64}$/)
  }
})
