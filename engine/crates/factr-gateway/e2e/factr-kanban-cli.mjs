#!/usr/bin/env node
// Exercise the desktop's Factr-owned Kanban store through its real CLI.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const factrRoot = process.env.FACTR_REPO || path.resolve('../backend')
const python = process.env.FACTR_PYTHON || path.join(factrRoot, '.venv/bin/python')
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'factr-kanban-e2e-'))
const env = {
  ...process.env,
  HOME: path.join(root, 'home'),
  FACTR_CONFIG_HOME: path.join(root, '.factr'),
  FACTR_HOME: path.join(root, '.factr/engine'),
  PYTHONDONTWRITEBYTECODE: '1',
}
for (const name of ['HOME', 'FACTR_CONFIG_HOME', 'FACTR_HOME']) fs.mkdirSync(env[name], { recursive: true })

function factr(...args) {
  const result = spawnSync(python, ['-m', 'factr_backend.main', 'kanban', ...args], {
    cwd: factrRoot,
    env,
    encoding: 'utf8',
    timeout: 30_000,
  })
  if (result.error || result.status !== 0) {
    throw new Error(`factr kanban ${args.join(' ')} failed: ${result.error || result.stderr || result.stdout}`)
  }
  return result.stdout.trim()
}

try {
  factr('init')
  factr('boards', 'create', 'factr-feature-e2e', '--name', 'Factr Feature E2E', '--switch')
  const created = JSON.parse(factr('create', 'Kanban CLI feature check', '--body', 'Persist and complete this task.', '--json'))
  assert.ok(created.id, 'CLI should return the created task id')
  assert.equal(created.title, 'Kanban CLI feature check')
  factr('claim', created.id)
  factr('complete', created.id, '--result', 'CLI e2e completed')
  const rows = JSON.parse(factr('list', '--json'))
  const saved = rows.find(task => task.id === created.id)
  assert.ok(saved, 'completed task should remain in the board')
  assert.equal(saved.status, 'done')
  assert.equal(saved.result, 'CLI e2e completed')
  console.log('PASS Factr Kanban CLI created a board, claimed a task, completed it, and read persisted state')
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}
