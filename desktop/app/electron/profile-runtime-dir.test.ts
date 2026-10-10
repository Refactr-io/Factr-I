import assert from 'node:assert/strict'
import path from 'node:path'

import { test } from 'vitest'

import { profileBackendRuntimeEnv, profileRuntimeDir } from './profile-runtime-dir'

const posix = { pathModule: path.posix, tmpdir: '/var/folders/ab/T', uid: 501 }

test('each profile backend gets its own runtime dir under the per-user base', () => {
  const work = profileRuntimeDir('work', { ...posix, env: {} })
  const other = profileRuntimeDir('other', { ...posix, env: {} })

  assert.match(work, /^\/var\/folders\/ab\/T\/factr-rt-[0-9a-f]{12}$/)
  assert.notEqual(work, other, 'two profiles never share a daemon lock')
  assert.equal(work, profileRuntimeDir('work', { ...posix, env: {} }), 'stable across restarts')
  assert.notEqual(work, profileRuntimeDir('work', { ...posix, env: {}, uid: 502 }), 'another user on a shared /tmp')
})

test('the base follows FACTR_RUNTIME_DIR, then XDG_RUNTIME_DIR, then TMPDIR', () => {
  assert.ok(profileRuntimeDir('p', { ...posix, env: { XDG_RUNTIME_DIR: '/run/user/1000' } }).startsWith('/run/user/1000/factr-rt-'))
  assert.ok(
    profileRuntimeDir('p', { ...posix, env: { FACTR_RUNTIME_DIR: '/rt', XDG_RUNTIME_DIR: '/run/user/1000' } }).startsWith('/rt/factr-rt-')
  )
})

test('a long profile name keeps the socket path short', () => {
  const dir = profileRuntimeDir('x'.repeat(200), { ...posix, env: {} })

  // The engine's socket is `<dir>/factr-<pid>.sock`; macOS caps the whole path near 104 bytes.
  assert.ok(`${dir}/factr-4194304.sock`.length < 104, dir)
})

test('the spawn env sets FACTR_RUNTIME_DIR and wins over an inherited one', () => {
  const inherited = { FACTR_RUNTIME_DIR: '/rt', PATH: '/usr/bin' }
  const env = { ...inherited, ...profileBackendRuntimeEnv('work', { ...posix, env: inherited }) }

  assert.equal(env.FACTR_RUNTIME_DIR, profileRuntimeDir('work', { ...posix, env: inherited }))
  assert.notEqual(env.FACTR_RUNTIME_DIR, '/rt')
  assert.equal(env.PATH, '/usr/bin')
})
