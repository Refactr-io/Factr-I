import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { test } from 'vitest'

import {
  buildPinArgs,
  buildPosixPinArgs,
  cleanInstallerLogLine,
  hasExistingGitCheckout,
  installedAgentInstallScript,
  isPinnedCommit,
  resolveInstallScript,
  resolveMarkerPinnedCommit,
  runBootstrap
} from './bootstrap-runner'

const SCRIPT_NAME = process.platform === 'win32' ? 'install.ps1' : 'install.sh'
const ZERO_COMMIT = '0000000000000000000000000000000000000000'

function mkTmpHome() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'factr-bootstrap-test-'))
}

test('runBootstrap bails immediately when the signal is already aborted', async () => {
  const controller = new AbortController()
  controller.abort()

  const events = []

  const result = await runBootstrap({
    installStamp: null,
    activeRoot: '/tmp/factr-runner-test',
    sourceRepoRoot: null,
    factrHome: '/tmp/factr-runner-test',
    logRoot: '/tmp/factr-runner-test',
    onEvent: ev => events.push(ev),
    abortSignal: controller.signal
  })

  // Cancelled before any install script is spawned.
  assert.deepEqual(result, { ok: false, cancelled: true })
  assert.ok(
    events.some(ev => ev.type === 'failed' && /cancelled/i.test(ev.error)),
    'should emit a cancelled failure event'
  )
})

test('installedAgentInstallScript resolves the installer in the agent checkout', () => {
  const home = mkTmpHome()

  try {
    assert.equal(installedAgentInstallScript(home), null, 'absent before the checkout exists')

    const scriptsDir = path.join(home, 'factr-backend', 'scripts')
    fs.mkdirSync(scriptsDir, { recursive: true })
    const scriptPath = path.join(scriptsDir, SCRIPT_NAME)
    fs.writeFileSync(scriptPath, '#!/bin/sh\necho hi\n')

    assert.equal(installedAgentInstallScript(home), scriptPath)
    assert.equal(installedAgentInstallScript(null), null, 'null home -> null')
  } finally {
    fs.rmSync(home, { recursive: true, force: true })
  }
})

test('existing checkout detection requires git metadata', () => {
  const home = mkTmpHome()

  try {
    const activeRoot = path.join(home, 'factr-backend')
    assert.equal(hasExistingGitCheckout(activeRoot), false)

    fs.mkdirSync(path.join(activeRoot, '.git'), { recursive: true })
    assert.equal(hasExistingGitCheckout(activeRoot), true)
  } finally {
    fs.rmSync(home, { recursive: true, force: true })
  }
})

test('fresh bootstrap args include the packaged commit pin', () => {
  const installStamp = { commit: 'a'.repeat(40), branch: 'main' }

  assert.deepEqual(buildPinArgs(installStamp), ['-Commit', installStamp.commit, '-Branch', 'main'])
  assert.deepEqual(
    buildPosixPinArgs({
      installStamp,
      activeRoot: '/tmp/factr-backend',
      factrHome: '/tmp/factr'
    }),
    ['--dir', '/tmp/factr-backend', '--factr-home', '/tmp/factr', '--branch', 'main', '--commit', installStamp.commit]
  )
})

test('existing-checkout bootstrap args keep branch but skip the packaged commit pin', () => {
  const installStamp = { commit: 'a'.repeat(40), branch: 'main' }

  assert.deepEqual(buildPinArgs(installStamp, { pinCommit: false }), ['-Branch', 'main'])
  assert.deepEqual(
    buildPosixPinArgs({
      installStamp,
      activeRoot: '/tmp/factr-backend',
      factrHome: '/tmp/factr',
      pinCommit: false
    }),
    ['--dir', '/tmp/factr-backend', '--factr-home', '/tmp/factr', '--branch', 'main']
  )
})

test('fallback install stamps pass a branch, never the zero commit', () => {
  const stamp = { commit: ZERO_COMMIT, branch: 'main' }

  assert.equal(isPinnedCommit(ZERO_COMMIT), false)
  // Must NOT pass -Commit / --commit for the all-zero placeholder.
  assert.deepEqual(buildPinArgs(stamp), ['-Branch', 'main'])
  assert.deepEqual(
    buildPosixPinArgs({
      installStamp: stamp,
      activeRoot: '/tmp/factr',
      factrHome: '/tmp/home'
    }),
    ['--dir', '/tmp/factr', '--factr-home', '/tmp/home', '--branch', 'main']
  )
})

test('resolveMarkerPinnedCommit prefers real HEAD over fallback stamp zeros', () => {
  const realHead = 'c'.repeat(40)
  assert.equal(
    resolveMarkerPinnedCommit({ commit: ZERO_COMMIT, branch: 'main' }, '/tmp/checkout', {
      resolveHead: () => realHead
    }),
    realHead
  )
  assert.equal(
    resolveMarkerPinnedCommit({ commit: 'd'.repeat(40), branch: 'main' }, '/tmp/checkout', {
      resolveHead: () => realHead
    }),
    'd'.repeat(40),
    'packaged real pin wins over checkout HEAD'
  )
  assert.equal(
    resolveMarkerPinnedCommit({ commit: ZERO_COMMIT, branch: 'main' }, '/tmp/missing', {
      resolveHead: () => null
    }),
    null
  )
})

test('resolveInstallScript uses the installed agent checkout and never downloads', async () => {
  const home = mkTmpHome()

  try {
    const scriptsDir = path.join(home, 'factr-backend', 'scripts')
    fs.mkdirSync(scriptsDir, { recursive: true })
    const installed = path.join(scriptsDir, SCRIPT_NAME)
    fs.writeFileSync(installed, '#!/bin/sh\necho installed\n')

    const result = await resolveInstallScript({
      installStamp: { commit: 'a'.repeat(40) },
      sourceRepoRoot: null,
      factrHome: home,
      emit: () => {}
    })

    assert.equal(result.source, 'installed-agent')
    assert.equal(result.path, installed)
    assert.equal(result.commit, 'a'.repeat(40))
  } finally {
    fs.rmSync(home, { recursive: true, force: true })
  }
})

test('resolveInstallScript rejects with the releases page when no local installer exists', async () => {
  const home = mkTmpHome()

  try {
    await assert.rejects(
      resolveInstallScript({
        installStamp: { commit: ZERO_COMMIT, branch: 'main' },
        sourceRepoRoot: null,
        factrHome: home,
        emit: () => {}
      }),
      /releases/
    )
  } finally {
    fs.rmSync(home, { recursive: true, force: true })
  }
})

// #112675: install.sh colours its banners and curl/uv redraw progress with \r
// even into a pipe; the overlay renders lines as plain text, so the emitter
// must hand every consumer (log ring, Details panel, Copy output) the text a
// terminal would be left showing.
test('installer log lines reach the emitter without escape sequences; \\r redraws keep the last frame', () => {
  assert.equal(cleanInstallerLogLine('\u001b[0;32m✓\u001b[0m Detected: macos (macos)'), '✓ Detected: macos (macos)')
  assert.equal(cleanInstallerLogLine('\u001b[2K\u001b[1GCloning repository…\u001b[K'), 'Cloning repository…')
  assert.equal(cleanInstallerLogLine('\u001b]0;factr\u0007Installing Factr'), 'Installing Factr')
  assert.equal(cleanInstallerLogLine('\r 12%\r 67%\r100%\u001b[K'), '100%')
  assert.equal(cleanInstallerLogLine('Resolving dependencies…\r'), 'Resolving dependencies…')
  // Only-escape frames drop entirely, so the caller emits nothing for them.
  assert.equal(cleanInstallerLogLine('\u001b[0m\r'), '')
  // Plain multi-byte text is untouched.
  assert.equal(cleanInstallerLogLine('Ready — café ✓ 中文'), 'Ready — café ✓ 中文')
})

test.skipIf(process.platform === 'win32')(
  'a manifest-step failure surfaces the installer tail without escape sequences',
  async () => {
    const home = mkTmpHome()
    fs.mkdirSync(path.join(home, 'scripts'))
    fs.writeFileSync(
      path.join(home, 'scripts', 'install.sh'),
      '#!/usr/bin/env bash\nprintf "\\033[0;31m\\xe2\\x9c\\x97\\033[0m manifest broke\\n" >&2\nexit 3\n'
    )

    const result = await runBootstrap({
      installStamp: null,
      activeRoot: home,
      sourceRepoRoot: home,
      factrHome: home,
      logRoot: home,
      onEvent: () => {}
    })

    assert.equal(result.ok, false)
    assert.equal(result.error, 'install.sh --manifest failed: exit 3\n✗ manifest broke')
  }
)
