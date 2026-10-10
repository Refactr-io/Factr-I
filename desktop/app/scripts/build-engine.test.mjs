import assert from 'node:assert/strict'
import { test } from 'vitest'

import { cflagsWithPrefixMaps, encodedRustflags, remapFlags } from './build-engine.mjs'

test('remaps home, cargo home, checkout and target dir, most specific last', () => {
  const flags = remapFlags({ home: '/Users/a b', cargoHome: '/Users/a b/.cargo', repoRoot: '/Users/a b/src/factr', targetDir: null })
  assert.deepEqual(flags, [
    '--remap-path-prefix=/Users/a b=~',
    '--remap-path-prefix=/Users/a b/.cargo=/cargo',
    '--remap-path-prefix=/Users/a b/src/factr=/factr'
  ])
  assert.equal(remapFlags({ home: '/h', cargoHome: '/c', repoRoot: '/r', targetDir: '/t' }).at(-1), '--remap-path-prefix=/t=/target')
})

test('keeps existing rustflags and paths with spaces as single arguments', () => {
  const remaps = ['--remap-path-prefix=/Users/a b=~']
  assert.equal(encodedRustflags({ RUSTFLAGS: ' -C  target-cpu=native ' }, remaps), ['-C', 'target-cpu=native', ...remaps].join('\x1f'))
  assert.equal(encodedRustflags({ CARGO_ENCODED_RUSTFLAGS: '-Cfoo=a b', RUSTFLAGS: '-Cignored' }, remaps), ['-Cfoo=a b', ...remaps].join('\x1f'))
  assert.equal(encodedRustflags({}, remaps), remaps[0])
})

test('maps the same prefixes for C sources, keeps existing CFLAGS, skips paths with spaces', () => {
  const remaps = ['--remap-path-prefix=/Users/a=~', '--remap-path-prefix=/Users/a b/.cargo=/cargo', '--remap-path-prefix=/t=/x']
  assert.equal(cflagsWithPrefixMaps(' -O2 ', remaps), '-O2 -ffile-prefix-map=/Users/a=~ -ffile-prefix-map=/t=/x')
  assert.equal(cflagsWithPrefixMaps(undefined, []), '')
})
