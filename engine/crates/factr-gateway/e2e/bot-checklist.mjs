#!/usr/bin/env node
// One entry point for the 17 upstream flows. Pending UI/external scope is explicit in the artifact.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const work = fileURLToPath(new URL('../../../../', import.meta.url))
const scratch = fs.mkdtempSync(path.join(process.env.SCRATCH || os.tmpdir(), 'bots-checklist-'))
const suites = ['bot-chat-registry.mjs', 'bot-chat-deliver.mjs', 'bot-runtime.mts', 'bot-schedules.mjs']
for (const name of suites) {
  const command = name.endsWith('.mts') ? path.join(work, 'desktop/node_modules/.bin/tsx') : process.execPath
  const log = fs.openSync(path.join(scratch, `${name}.log`), 'w')
  console.log(`Running ${name}`)
  const child = spawn(command, [fileURLToPath(new URL(name, import.meta.url))], { cwd: work, env: { ...process.env, SCRATCH: scratch }, stdio: ['ignore', log, log] })
  const timer = setTimeout(() => child.kill('SIGKILL'), 15 * 60_000)
  const result = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', (code, signal) => resolve({ code, signal })) })
  clearTimeout(timer); fs.closeSync(log)
  assert.equal(result.code, 0, `${name} failed (${result.signal || result.code}); see ${scratch}`)
}

const cases = [
  ['Create and intro', 'bot-chat-deliver', 'GUI: New Agent controls and first rendered intro'],
  ['Advanced, edit, clone and delete', 'bot-schedules', 'GUI: confirmation, model/skill/MCP pickers and Description editing'],
  ['Canonical chat and /new', 'bot-chat-registry', 'GUI: /new rewrites to /compact with notice'],
  ['Default @mention', 'bot-runtime + bot-chat-deliver', 'GUI: autocomplete and attribution rendering'],
  ['Bot to bot, roster refusal and wake', 'bot-runtime + bot-chat-deliver', 'GUI: both transcripts'],
  ['SILENT', 'bot-chat-deliver', 'GUI: marker remains invisible'],
  ['Auth reason and transient retry', 'bot-chat-deliver', 'GUI: typed reason displayed'],
  ['Groups', 'bot-runtime (member session, turn, restart)', 'PARTIAL: room-round coordinator remains in Desktop; no close-Desktop durable driver'],
  ['Routines create/fire/pause/resume/delete', 'bot-schedules', 'GUI: natural-language schedule and Run now control'],
  ['Scheduled bot-chat delivery', 'bot-schedules (own canonical engine chat)', 'PARTIAL: explicit other-profile delivery needs courier integration'],
  ['Cron cannot self-schedule', 'bot-schedules (model tool policy)', 'GUI: model refusal wording; external tool attempts remain denied by existing policy'],
  ['Parallel children and Stop', 'bot-runtime', 'GUI: child status and partial transcript'],
  ['Second machine', 'bot-runtime (relay protocol only)', 'EXTERNAL: physical second-machine connection and Create on'],
  ['Registered peer', 'bot-runtime (Python HTTP golden + stub gateway)', 'EXTERNAL: real peer registration and account'],
  ['Per-bot Telegram accounts', null, 'EXTERNAL: two disposable bot accounts and duplicate-token lock'],
  ['Computer screen/takeover/hand-back', null, 'GUI/DOCKER: computer integration checklist'],
  ['Live Bot Mode toggle', 'bot-runtime (native protocol gate)', 'GUI: live capability registration and retained data']
].map(([flow, engineEvidence, remaining], index) => ({ item: index + 1, flow, engineEvidence, remaining }))
assert.equal(cases.length, 17)
const output = { suites, cases, note: 'Suite success is engine evidence only. Remaining scope is not counted as passed.' }
fs.writeFileSync(path.join(scratch, 'checklist.json'), JSON.stringify(output, null, 2) + '\n')
console.log(`PASS four engine suites; all 17 flows accounted for with explicit limits. Artifact: ${scratch}/checklist.json`)
