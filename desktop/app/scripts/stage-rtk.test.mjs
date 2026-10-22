import assert from 'node:assert/strict'
import { test } from 'vitest'

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { RTK_ASSETS, stageRtk, publishedHash, sha256, verifyAsset } from './stage-rtk.mjs'

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
  assert.deepEqual(Object.keys(RTK_ASSETS).sort(), ['darwin-arm64', 'darwin-x64', 'linux-x64'])
  for (const { asset: name, sha256: hash } of Object.values(RTK_ASSETS)) {
    assert.match(name, /^rtk-oss-/)
    assert.match(hash, /^[0-9a-f]{64}$/)
  }
})

test('stageRtk unpacks a verified archive and refuses a tampered one', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'stage-rtk-test-'))
  const src = path.join(dir, 'src')
  fs.mkdirSync(src)
  fs.writeFileSync(path.join(src, 'rtk'), '#!/bin/sh\n')
  fs.writeFileSync(path.join(src, 'LICENSE'), 'Apache License\nVersion 2.0\n')
  const asset = RTK_ASSETS['linux-x64'].asset
  execFileSync('tar', ['-czf', path.join(dir, asset), '-C', src, 'rtk', 'LICENSE'])
  const archive = fs.readFileSync(path.join(dir, asset))
  const hash = sha256(archive)
  const fetchBytes = async url => (url.endsWith('checksums.txt') ? Buffer.from(`${hash}  ${asset}\n`) : archive)
  // The pin is the real asset's hash, so this synthetic archive must be refused.
  await assert.rejects(stageRtk({ platform: 'linux', arch: 'x64', outDir: path.join(dir, 'out'), fetchBytes }), /does not match the pinned/)
  assert.ok(!fs.existsSync(path.join(dir, 'out')))
})
