import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { test } from 'vitest'

import {
  archiveArgs,
  badSymlinks,
  compileallArgs,
  integrityPath,
  makeUserWritable,
  packagedDefaults,
  pycFor,
  snapshotIntegrity,
  sourcesMissingPyc,
  verifyIntegrity
} from './stage-backend-python.mjs'

const tree = () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'stage-py-'))
  const write = (rel, body = '') => {
    fs.mkdirSync(path.dirname(path.join(root, rel)), { recursive: true })
    fs.writeFileSync(path.join(root, rel), body)
  }
  return { root, write }
}

test('importing the stage script does not stage anything', () => {
  assert.equal(typeof badSymlinks, 'function')
})

test('pycFor names the default-optimization CPython 3.12 cache file', () => {
  assert.equal(pycFor(path.join('a', 'b', 'mod.py')), path.join('a', 'b', '__pycache__', 'mod.cpython-312.pyc'))
})

test('sourcesMissingPyc lists .py files with no precompiled pyc', () => {
  const { root, write } = tree()
  write('source/pkg/a.py')
  write('source/pkg/__pycache__/a.cpython-312.pyc')
  write('source/pkg/b.py')
  write('source/pkg/notes.txt')
  assert.deepEqual(sourcesMissingPyc(root), [path.join('source', 'pkg', 'b.py')])
  fs.rmSync(root, { recursive: true, force: true })
})

test('verifyIntegrity passes an untouched tree and catches later writes, new pycs and pyc gaps', () => {
  const { root, write } = tree()
  write('source/a.py')
  write('source/__pycache__/a.cpython-312.pyc')
  write('runtime/lib/bad.py')
  const snap = snapshotIntegrity(root)
  assert.deepEqual(snap.uncompilable, [path.join('runtime', 'lib', 'bad.py')])
  assert.deepEqual(verifyIntegrity(root, snap), [])

  // A runtime import writing bytecode into the bundle: new file, newer mtime.
  write('packages/x/__pycache__/y.cpython-312.pyc')
  const later = new Date(snap.stampedAt + 5000)
  fs.utimesSync(path.join(root, 'packages/x/__pycache__/y.cpython-312.pyc'), later, later)
  const problems = verifyIntegrity(root, snap)
  assert.ok(problems.some(p => p.includes('modified after staging')), problems.join('\n'))
  assert.ok(problems.some(p => p.includes('file count changed')), problems.join('\n'))

  // A compiled source whose pyc went missing.
  fs.rmSync(path.join(root, 'packages'), { recursive: true })
  fs.rmSync(path.join(root, 'source/__pycache__'), { recursive: true })
  assert.ok(verifyIntegrity(root, snap).some(p => p.includes('without a precompiled pyc')))
  fs.rmSync(root, { recursive: true, force: true })
})

test('makeUserWritable adds u+w to read-only files and dirs', { skip: process.platform === 'win32' }, () => {
  const { root, write } = tree()
  write('tools/uv', 'bin')
  fs.chmodSync(path.join(root, 'tools/uv'), 0o555)
  fs.chmodSync(path.join(root, 'tools'), 0o555)
  makeUserWritable(root)
  assert.equal(fs.statSync(path.join(root, 'tools/uv')).mode & 0o777, 0o755)
  assert.equal(fs.statSync(path.join(root, 'tools')).mode & 0o200, 0o200)
  fs.rmSync(root, { recursive: true, force: true })
})

test('compileall writes unchecked-hash pycs with the build path stripped', () => {
  const args = compileallArgs('/b/stage', ['runtime', 'source'])
  assert.deepEqual(args.slice(0, 3), ['-m', 'compileall', '-q'])
  assert.ok(args.includes('-f'))
  assert.equal(args[args.indexOf('--invalidation-mode') + 1], 'unchecked-hash')
  assert.equal(args[args.indexOf('-s') + 1], '/b/stage')
  assert.deepEqual(args.slice(-2), [path.join('/b/stage', 'runtime'), path.join('/b/stage', 'source')])
})

test('the backend test suite is left out of the staged source', () => {
  assert.deepEqual(archiveArgs().slice(-2), ['HEAD:backend', ':(exclude)tests'])
})

test('packaged defaults stop borrowing other CLI logins without touching the rest', () => {
  const cli = { auth: { adopt_external_logins: true, codex_login: 'device_code' }, model: 'x' }
  const packaged = packagedDefaults(cli)
  assert.equal(packaged.auth.adopt_external_logins, false)
  assert.equal(packaged.auth.codex_login, 'device_code')
  assert.equal(packaged.model, 'x')
  assert.equal(cli.auth.adopt_external_logins, true, 'the CLI defaults object is not mutated')
  assert.equal(packagedDefaults({}).auth.adopt_external_logins, false)
})

test('the integrity snapshot lives beside the staged tree, not inside the bundle', () => {
  assert.equal(integrityPath('/x/build/backend-python'), '/x/build/backend-python.integrity.json')
})
