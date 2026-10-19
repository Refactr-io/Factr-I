// Real Linux confinement check. Run through test_local_sandbox_linux.sh.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { spawn, spawnSync } from 'node:child_process';
import { prepareBotSandbox } from './launcher.mjs';

const base = fs.mkdtempSync('/tmp/bots-linux-');
const root = path.join(base, 'profiles/alpha');
fs.mkdirSync(root, { recursive: true });
const canary = path.join(base, 'host-canary');
fs.writeFileSync(canary, 'HOST_ONLY');
fs.symlinkSync(canary, path.join(root, 'escape'));
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(test, ms = 30000) {
  const deadline = Date.now() + ms;
  while (!test()) { assert.ok(Date.now() < deadline, 'Linux engine readiness timeout'); await sleep(50); }
}
let child, engineLaunch, model;
try {
  const code = `echo OWN > own.txt; cat own.txt; cat '${canary}' 2>/dev/null || echo HOST_DENIED; cat ../escape 2>/dev/null || echo LINK_DENIED; (echo bad > '${canary}') 2>/dev/null || echo WRITE_DENIED; test ! -e /host && echo NO_HOST_MOUNT; printf 'PIDNS='; readlink /proc/self/ns/pid`;
  const shell = prepareBotSandbox({ profile: 'alpha', configHome: base, command: '/bin/sh', args: ['-c', code], env: process.env, readRoots: [] });
  try {
    const result = spawnSync(shell.command, shell.args, { cwd: shell.cwd, env: shell.env, encoding: 'utf8', timeout: 15000 });
    assert.equal(result.status, 0, result.stderr);
    for (const key of ['OWN', 'HOST_DENIED', 'LINK_DENIED', 'NO_HOST_MOUNT']) assert.ok(result.stdout.includes(key), result.stdout);
    assert.ok(!result.stdout.includes('PIDNS=' + fs.readlinkSync('/proc/self/ns/pid')), 'the bot must have a separate PID namespace');
    assert.equal(fs.readFileSync(canary, 'utf8'), 'HOST_ONLY', 'a write to private /tmp must not modify the host file');
    assert.equal(fs.readFileSync(path.join(root, 'workspace/own.txt'), 'utf8'), 'OWN\n');
    console.log('PASS real Linux bubblewrap files, private root/temp, symlink boundary and PID namespace');
  } finally { shell.cleanup(); }

  model = http.createServer(async (req, res) => {
    let raw = ''; for await (const data of req) raw += data;
    res.setHeader('content-type', 'application/json');
    if (req.method === 'GET') return res.end(JSON.stringify({ data: [{ id: 'linux-stub' }] }));
    const body = JSON.parse(raw), messages = body.messages || [];
    const done = messages.some(m => m.role === 'tool') && messages.some(m => m.tool_calls?.some(t => t.function.name === 'bash'));
    const offered = body.tools?.some(t => t.function?.name === 'bash');
    const args = offered ? { command: `echo ENGINE_FILE > engine.txt; cat engine.txt; cat '${canary}' 2>/dev/null || echo ENGINE_DENIED`, timeout: 10000 } : { names: ['bash'] };
    const last = [...messages].reverse().find(m => m.role === 'tool');
    const message = done ? { role: 'assistant', content: `RESULT ${last.content}` } : { role: 'assistant', tool_calls: [{ id: 'linux-tool-' + messages.length, type: 'function', function: { name: offered ? 'bash' : 'load_tools', arguments: JSON.stringify(args) } }] };
    if (!body.stream) return res.end(JSON.stringify({ id: 'linux', choices: [{ message, finish_reason: done ? 'stop' : 'tool_calls' }] }));
    res.setHeader('content-type', 'text/event-stream');
    for (const [delta, finish_reason] of [[message, null], [{}, done ? 'stop' : 'tool_calls']]) {
      if (delta.tool_calls) delta.tool_calls[0].index = 0;
      res.write(`data: ${JSON.stringify({ id: 'linux', object: 'chat.completion.chunk', model: 'linux-stub', choices: [{ index: 0, delta, finish_reason }] })}\n\n`);
    }
    res.end('data: [DONE]\n\n');
  });
  await new Promise(resolve => model.listen(0, '127.0.0.1', resolve));
  fs.writeFileSync(path.join(root, 'config.yaml'), `model:\n  provider: custom\n  default: linux-stub\n  base_url: http://127.0.0.1:${model.address().port}/v1\n  api_key: none\napprovals:\n  mode: 'off'\nplugins:\n  enabled: []\n`);
  fs.writeFileSync(path.join(root, 'profile.yaml'), 'ui_meta:\n  factr-bots:\n    title: Alpha\n');
  const python = '/opt/factr/backend-python';
  const token = 'synthetic-linux-sandbox-test-token';
  engineLaunch = prepareBotSandbox({ profile: 'alpha', configHome: base, command: '/usr/local/bin/factr',
    args: ['--profile', 'alpha', 'serve', '--host', '127.0.0.1', '--port', '0', '--token-stdin'],
    env: { ...process.env, FACTR_BOT_COMPUTER: '0', FACTR_REPL: '0', OLLAMA_HOST: '127.0.0.1:1',
      FACTR_BACKEND_PYTHON: `${python}/runtime/bin/python3.12`, FACTR_BACKEND_PYTHONPATH: `${python}/source:${python}/packages`,
      FACTR_BACKEND_TOOLS_DIR: `${python}/source/tools`, PYTHONDONTWRITEBYTECODE: '1' }, readRoots: [python] });
  child = spawn(engineLaunch.command, engineLaunch.args, { cwd: engineLaunch.cwd, env: engineLaunch.env, stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', data => { output += data; }); child.stderr.on('data', data => { output += data; });
  child.stdin.end(token + '\n');
  await until(() => /FACTR_BACKEND_READY port=\d+/.test(output) || child.exitCode !== null || child.signalCode !== null);
  assert.equal(child.exitCode, null, output); assert.equal(child.signalCode, null, output);
  const port = output.match(/FACTR_BACKEND_READY port=(\d+)/)[1];
  const headers = { authorization: `Bearer ${token}`, 'content-type': 'application/json' };
  const response = await fetch(`http://127.0.0.1:${port}/api/agent/run`, { method: 'POST', headers,
    body: JSON.stringify({ prompt: 'LINUX_ENGINE_BOUNDARY', surface: 'bot', title: 'Bot Chat', timeout_s: 30 }), signal: AbortSignal.timeout(45000) });
  const result = await response.json();
  assert.equal(result.ok, true, JSON.stringify(result) + output);
  for (const marker of ['ENGINE_FILE', 'ENGINE_DENIED']) assert.ok(JSON.stringify(result).includes(marker), JSON.stringify(result));
  assert.equal(fs.readFileSync(path.join(root, 'workspace/engine.txt'), 'utf8'), 'ENGINE_FILE\n');
  assert.equal(fs.readFileSync(canary, 'utf8'), 'HOST_ONLY');
  const feature = await fetch(`http://127.0.0.1:${port}/api/cron/jobs?feature=1&profile=alpha`, { headers, signal: AbortSignal.timeout(45000) });
  assert.equal(feature.status, 200, await feature.text() + output);
  console.log('PASS real Linux engine turn, native shell boundary and bundled Python feature backend');
} finally {
  if (child) { child.kill('SIGTERM'); await until(() => child.exitCode !== null || child.signalCode !== null, 10000).catch(() => child.kill('SIGKILL')); }
  engineLaunch?.cleanup();
  if (model) { model.closeAllConnections(); model.close(); }
  fs.rmSync(base, { recursive: true, force: true });
}
