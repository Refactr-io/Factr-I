// Real engines, SDK message_agent, the production pool arbiter, and a deterministic local model.
// Run from the worktree with desktop/node_modules/.bin/tsx and FACTR_BIN set.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createPoolRetirer } from "../../../../desktop/app/electron/pool-retire.ts";
import { createPoolRetirementClient } from "../../../../desktop/app/electron/pool-retire-http.ts";
import { LocalBackendSpawnCoordinator } from "../../../../desktop/app/electron/pool-spawn-coordinator.ts";

const work = fileURLToPath(new URL("../../../../", import.meta.url));
const base = fs.mkdtempSync(
  path.join(process.env.SCRATCH || os.tmpdir(), "bots-runtime-"),
);
const cfg = path.join(base, "config"),
  home = path.join(base, "engine");
const children = new Set<any>();
const seen: any[] = [];
const peerRequests: any[] = [];
const closedChildren: string[] = [];
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(test: () => boolean, ms = 15_000) {
  const end = Date.now() + ms;
  while (!test()) {
    assert.ok(Date.now() < end, "condition timed out");
    await sleep(25);
  }
}
const model = http.createServer(async (req, res) => {
  let raw = "";
  for await (const chunk of req) raw += chunk;
  if (req.method === "GET") {
    res.setHeader("content-type", "application/json");
    return res.end(JSON.stringify({ data: [{ id: "bot-stub" }] }));
  }
  const body = JSON.parse(raw),
    messages = body.messages || [];
  const contentText = (m: any) => typeof m?.content === "string" ? m.content : (m?.content || []).map((p: any) => p.text || "").join("");
  const last = [...messages].reverse().find((m) => m.role === "user" && !contentText(m).includes("Teammates and roles:"));
  const user =
    typeof last?.content === "string"
      ? last.content
      : (last?.content || []).map((p) => p.text || "").join("");
  const entry = { user, body, at: Date.now() };
  seen.push(entry);
  const lastTool = [...messages].reverse().find(m => m.role === "tool");
  const calls = (user.includes("NATIVE_ASK") || user.includes("NATIVE_PEER") || user.includes("NATIVE_HEADLESS")) && !lastTool;
  if (user.includes("SLOW_DELIVERY")) await sleep(3000);
  const text = calls
    ? ""
    : lastTool && user.includes("NATIVE_ASK")
      ? `Tool acknowledged: ${lastTool.content}`
      : `Answer: ${user}`;
  res.setHeader("content-type", "text/event-stream");
  const chunk = (delta: any, finish: string | null) =>
    `data: ${JSON.stringify({ id: "stub", object: "chat.completion.chunk", created: 1, model: "bot-stub", choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;
  if (user.includes('DELEGATE_PARENT') && !user.includes('CHILD_WAIT_')) {
    const loaded = body.tools?.some(t => t.function.name === 'delegate');
    const spawned = messages.some(m => m.tool_calls?.some(t => t.function.name === 'delegate'));
    const tool = (name: string, args: any, index: number) => ({ index, id: `task-${index}-${name}`, type: 'function', function: { name, arguments: JSON.stringify(args) } });
    const tasks = !loaded ? [tool('load_tools', { names: ['delegate'] }, 0)] : !spawned ? [
      tool('delegate', { action: 'spawn', label: 'one', prompt: 'CHILD_WAIT_A' }, 0),
      tool('delegate', { action: 'spawn', label: 'two', prompt: 'CHILD_WAIT_B' }, 1)
    ] : [];
    res.write(chunk(tasks.length ? { role: 'assistant', tool_calls: tasks } : { role: 'assistant', content: 'Children started' }, null));
    res.write(chunk({}, tasks.length ? 'tool_calls' : 'stop'));
    return res.end('data: [DONE]\n\n');
  }
  if (user.includes('CHILD_WAIT_')) {
    res.write(chunk({ role: 'assistant', content: 'Partial child work' }, null));
    await new Promise<void>(resolve => { const timer = setTimeout(resolve, 60000); res.once('close', () => { closedChildren.push(user); clearTimeout(timer); resolve() }); });
    if (!res.destroyed) res.end(chunk({}, 'stop') + 'data: [DONE]\n\n');
    return;
  }
  res.write(
    chunk(
      calls
        ? {
            role: "assistant",
            tool_calls: [
              {
                index: 0,
                id: "handoff",
                type: "function",
                function: {
                  name: "message_agent",
                  arguments: JSON.stringify({
                    target: user.includes("NATIVE_PEER") ? "spark/researcher" : user.includes("NATIVE_HEADLESS") ? "a" : "b",
                    message: "SLOW_DELIVERY native ping",
                  }),
                },
              },
            ],
          }
        : { role: "assistant", content: text },
      null,
    ),
  );
  res.write(chunk({}, calls ? "tool_calls" : "stop"));
  res.end("data: [DONE]\n\n");
});
await new Promise<void>((resolve) => model.listen(0, "127.0.0.1", resolve));
const peer = http.createServer(async (req, res) => {
  let raw = ''; for await (const part of req) raw += part;
  const body = raw ? JSON.parse(raw) : undefined;
  peerRequests.push({ method: req.method, url: req.url, body });
  assert.equal(req.headers.authorization, 'Bearer bots-peer-test');
  res.setHeader('content-type', 'application/json');
  if (req.method === 'GET') return res.end(JSON.stringify({ data: [{ id: 'hidden-chat', title: 'Bot Chat', hidden: true }] }));
  res.end(JSON.stringify({ message: { content: 'PEER_ANSWER' } }));
});
await new Promise<void>(resolve => peer.listen(0, '127.0.0.1', resolve));
const endpoint = `http://127.0.0.1:${(model.address() as any).port}/v1`;
fs.mkdirSync(home, { recursive: true });
fs.mkdirSync(cfg, { recursive: true });
fs.writeFileSync(
  path.join(home, "config.toml"),
  `[provider]\ndefault_provider="local"\n[providers.local]\ntype="openai-compatible"\nbase_url="${endpoint}"\napi_key="none"\nrequires_api_key=false\ndefault_model="bot-stub"\n[[providers.local.models]]\nid="bot-stub"\ncontext_window=8192\n`,
);
function seed(name: string) {
  const dir = name === "default" ? cfg : path.join(cfg, "profiles", name);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, "config.yaml"),
    `model:\n  provider: custom\n  default: bot-stub\n  base_url: ${endpoint}\n  api_key: none\n`,
  );
  if (name !== "default")
    fs.writeFileSync(
      path.join(dir, "profile.yaml"),
      `description: Test teammate ${name}\nui_meta:\n  factr-bots:\n    title: ${name}\n`,
    );
}
seed("default");
fs.appendFileSync(path.join(cfg, 'config.yaml'), `bot_peers:\n  spark:\n    url: http://127.0.0.1:${(peer.address() as any).port}\n`);
async function start(name: string) {
  const token = crypto.randomBytes(24).toString("hex");
  const userHome = path.join(base, "homes", name);
  fs.mkdirSync(userHome, { recursive: true });
  const child = spawn(
    process.env.FACTR_BIN || path.join(work, "target/debug/factr"),
    ["--profile", name, "serve", "--host", "127.0.0.1", "--port", "0"],
    {
      cwd: userHome,
      env: {
        ...process.env,
        HOME: userHome,
        FACTR_CONFIG_HOME: cfg,
        FACTR_HOME: home,
        FACTR_RUNTIME_DIR: fs.mkdtempSync("/private/tmp/bots-rpc-"),
        FACTR_DASHBOARD_SESSION_TOKEN: token,
        OLLAMA_HOST: "127.0.0.1:1",
        FACTR_PEER_SPARK_KEY: 'bots-peer-test',
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  children.add(child);
  child.once("exit", () => children.delete(child));
  let out = "",
    err = "";
  child.stdout.on("data", (d) => {
    out += d;
  });
  child.stderr.on("data", (d) => {
    err += d;
  });
  await until(
    () => /FACTR_BACKEND_READY port=\d+/.test(out) || child.exitCode !== null,
  );
  assert.equal(child.exitCode, null, err);
  const port = Number(out.match(/FACTR_BACKEND_READY port=(\d+)/)![1]);
  const sockets = new Set<WebSocket>();
  const open = async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws?token=${token}`);
    sockets.add(ws);
    let n = 0;
    const pending = new Map<any, any>(),
      events: any[] = [];
    ws.addEventListener("message", (e) => {
      const f = JSON.parse(String(e.data));
      if (pending.has(f.id)) pending.get(f.id)(f);
      else if (f.method === "event") events.push(f.params);
    });
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener("open", () => resolve());
      ws.addEventListener("error", reject);
    });
    return {
      events,
      close: () => {
        ws.close();
        sockets.delete(ws);
      },
      raw: (method: string, params = {}) =>
        new Promise<any>((resolve, reject) => {
          const id = ++n,
            timer = setTimeout(() => {
              pending.delete(id);
              reject(
                new Error(`${name}: ${method} timeout\n${err.slice(-1000)}`),
              );
            }, 70_000);
          pending.set(id, (f) => {
            clearTimeout(timer);
            pending.delete(id);
            resolve(f);
          });
          ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
        }),
    };
  };
  const rpc = await open();
  const call = async (method: string, params = {}) => {
    const f = await rpc.raw(method, params);
    assert.equal(f.error, undefined, JSON.stringify(f.error));
    return f.result;
  };
  const stop = async () => {
    for (const ws of sockets) ws.close();
    child.kill("SIGTERM");
    const force = setTimeout(() => child.kill("SIGKILL"), 5000);
    await until(
      () => child.exitCode !== null || child.signalCode !== null,
      7000,
    );
    clearTimeout(force);
  };
  const retirement = async (action: string, permit?: string) => {
    const r = await fetch(`http://127.0.0.1:${port}/api/health/retirement`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
      },
      body: JSON.stringify({ action, ...(permit ? { token: permit } : {}) }),
    });
    assert.equal(r.status, 200);
    return r.json() as Promise<any>;
  };
  const run = async (body: any) => {
    const response = await fetch(`http://127.0.0.1:${port}/api/agent/run`, { method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: JSON.stringify(body) });
    assert.equal(response.status, 200, await response.clone().text());
    return response.json() as Promise<any>;
  };
  return {
    ...rpc,
    call,
    open,
    stop,
    retirement,
    run,
    child,
    process: child,
    port,
    token,
    kind: "factr",
    lastActiveAt: Date.now(),
    activeTurn: false,
  };
}
let primary: Awaited<ReturnType<typeof start>>;
const pool = new Map<string, Awaited<ReturnType<typeof start>>>();
const coordinator = new LocalBackendSpawnCoordinator(3);
const releases = new Map<string, () => void>();
const stopped: string[] = [];
const retirer = createPoolRetirer({
  pool,
  coordinator,
  ...createPoolRetirementClient(async (url, token, options) => {
    const res = await fetch(url, {
      method: options.method,
      headers: {
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(options.body),
    });
    assert.equal(res.status, 200);
    return res.json();
  }),
  stopBackend: async (key) => {
    const engine = pool.get(key)!;
    await engine.stop();
    assert.ok(
      engine.child.exitCode !== null || engine.child.signalCode !== null,
    );
    pool.delete(key);
    stopped.push(key);
    releases.get(key)!();
  },
});
async function pooled(name: string) {
  await retirer.waitForRetirement(name);
  if (pool.has(name)) return pool.get(name)!;
  const release = await coordinator.request(name, {
    priority: "foreground",
    timeoutMs: 20_000,
  }).acquired;
  releases.set(name, release);
  try {
    const engine = await start(name);
    pool.set(name, engine);
    assert.ok(coordinator.activeCount <= 3);
    return engine;
  } catch (err) {
    release();
    throw err;
  }
}
async function delivery(
  engine: Awaited<ReturnType<typeof start>>,
  params: any,
) {
  let r = await engine.call("bot.chat.deliver", params);
  while (r.status === "pending")
    r = await engine.call("bot.chat.deliver_status", {
      envelope_id: params.envelope_id,
    });
  return r;
}
try {
  console.log("stage: default engine");
  primary = await start("default");
  const chat = await primary.call("session.create", {
    title: "Existing default chat",
  });
  await primary.call("prompt.submit", {
    session_id: chat.session_id,
    text: "before roster",
  });
  await until(() =>
    primary.events.some(
      (e) => e.session_id === chat.session_id && e.type === "message.complete",
    ),
  );
  for (const name of ["a", "b", "c", "d", "e"]) seed(name);
  console.log("stage: target engine");
  const b = await pooled("b");
  const targetChat = (await b.call("bot.chat.ensure", { profile: "b" }))
    .canonical_session.id;
  const groups = await Promise.all([b.call('bot.group.ensure', { profile: 'b', title: 'Group: runtime-room · thread' }), b.call('bot.group.ensure', { profile: 'b', title: 'Group: runtime-room · thread' })]);
  assert.equal(groups[0].session_id, groups[1].session_id, 'concurrent opens share one member session');
  const groupId = groups[0].session_id;
  await b.call('prompt.submit', { session_id: groupId, text: 'GROUP_MEMBER_TURN' });
  await until(() => b.events.some(e => e.session_id === groupId && e.type === 'message.complete'));
  assert.equal((await b.call('bot.chat.get', { profile: 'b' })).canonical_session.id, targetChat);
  const oldRoutine = '[bot-mode:routine:v2] You are running a routine\n\nfactr -p \'b\' chat -c \'Routine: Check\' -q \'[Scheduled routine] ROUTINE_LITERAL $(keep) `keep`\'\n\nIf the command fails, report the error instead.';
  const routine = await b.run({ prompt: oldRoutine, surface: 'cron', title: '[bot:b] Check', timeout_s: 30 });
  assert.equal(routine.ok, true, JSON.stringify(routine));
  assert.ok(seen.some(s => s.user === '[Scheduled routine] ROUTINE_LITERAL $(keep) `keep`'));
  assert.ok(!seen.some(s => s.user.includes('factr -p')));
  console.log('ok: member group session and saved routine execute through the owner engine');
  const parent = await primary.call('session.create', { title: 'Stop family' });
  await primary.call('prompt.submit', { session_id: parent.session_id, text: 'DELEGATE_PARENT' });
  await until(() => seen.some(s => s.user.includes('CHILD_WAIT_A')) && seen.some(s => s.user.includes('CHILD_WAIT_B')), 30000);
  console.log('family before Stop', JSON.stringify(await primary.call('subagent.list', { session_id: parent.session_id })));
  const stoppedFamily = await primary.call('session.interrupt', { session_id: parent.session_id });
  if (!stoppedFamily.children_stopped) console.log('Stop diagnostic', JSON.stringify(primary.events.filter(e => e.type === 'error')));
  assert.ok(stoppedFamily.children_stopped >= 2, JSON.stringify(stoppedFamily));
  await until(() => closedChildren.length >= 2, 10000);
  console.log('ok: both child provider streams closed after Stop');
  let childRows = await primary.call('subagent.list', { session_id: parent.session_id });
  for (let i=0; i<100 && (childRows.subagents || []).some(c => c.status === 'running'); i++) {
    await sleep(100);
    childRows = await primary.call('subagent.list', { session_id: parent.session_id });
  }
  assert.ok(!(childRows.subagents || []).some(c => c.status === 'running'), JSON.stringify(childRows));
  console.log('ok: Stop cancels two real delegated children');
  const headless = await b.run({ prompt: "NATIVE_HEADLESS", surface: "bot", title: "Bot Chat", session_key: "headless-title-regression", timeout_s: 30 });
  assert.equal(headless.ok, true, JSON.stringify(headless));
  const headlessCall = [...seen].reverse().find(s => s.user.includes("NATIVE_HEADLESS") && s.body.messages.some(m => m.role === "tool"));
  assert.ok(headlessCall && JSON.stringify(headlessCall.body.messages).includes('queued'), "headless Bot Chat can queue native message_agent");
  assert.ok(!JSON.stringify(headlessCall.body.messages).includes('disabled in this session'));
  const storedHeadless = JSON.parse(fs.readFileSync(path.join(home, "sessions", `${headless.session_id}.json`), "utf8"));
  assert.equal(storedHeadless.custom_title || storedHeadless.title, 'Bot Chat');
  for (const file of fs.readdirSync(path.join(cfg, 'bot_relay/outbox'))) {
    const pending = JSON.parse(fs.readFileSync(path.join(cfg, 'bot_relay/outbox', file), 'utf8'));
    if (pending.sender_session === headless.session_id) fs.unlinkSync(path.join(cfg, 'bot_relay/outbox', file));
  }
  console.log("ok: first headless Bot Chat retains its title and queues native messaging");
  console.log("stage: native handoff");
  await primary.call("prompt.submit", {
    session_id: chat.session_id,
    text: "NATIVE_ASK",
  });
  await until(
    () =>
      fs.existsSync(path.join(cfg, "bot_relay/outbox")) &&
      fs
        .readdirSync(path.join(cfg, "bot_relay/outbox"))
        .some((s) => s.endsWith(".json")),
  );
  const queuedAt = Date.now(),
    file = fs
      .readdirSync(path.join(cfg, "bot_relay/outbox"))
      .find((s) => s.endsWith(".json"))!;
  const envelope = JSON.parse(
    fs.readFileSync(path.join(cfg, "bot_relay/outbox", file), "utf8"),
  );
  await until(() =>
    seen.some(
      (s) => s.user.includes("NATIVE_ASK") && s.body.messages.some(m => m.role === "tool"),
    ),
  );
  assert.ok(
    Date.now() - queuedAt < 2000,
    "the tool acknowledges before B is even run",
  );
  assert.ok(
    seen
      .find((s) => s.user.includes("NATIVE_ASK"))
      ?.body.tools.some((t) => t.function.name === "message_agent"),
    "existing default session gains the native tool",
  );
  console.log("stage: relay target");
  const task = delivery(b, {
    profile: "b",
    text: envelope.message,
    envelope_id: envelope.id,
    hops: envelope.hops,
  });
  await until(() => seen.some((s) => s.user.includes("SLOW_DELIVERY")));
  assert.equal(
    (await b.retirement("prepare")).idle,
    false,
    "a running real engine is not retired",
  );
  const result = await task;
  assert.equal(result.ok, true);
  console.log("stage: sender wake");
  const notified = await primary.call("bot.chat.reply", {
    id: envelope.id,
    reply: result.reply,
  });
  assert.equal(notified.ok, true);
  assert.ok(
    seen.some((s) => s.user.startsWith("Reply from @b:")),
    "the later reply wakes the original sender session",
  );
  const duplicate = await primary.call("bot.chat.reply", {
    id: envelope.id,
    reply: result.reply,
  });
  assert.equal(duplicate.reply, notified.reply);
  assert.equal(
    seen.filter((s) => s.user.startsWith("Reply from @b:")).length,
    1,
  );
  // A reply/expiry written by the existing Python courier must survive the sender process.
  await primary.stop();
  const expiredId = crypto.randomBytes(16).toString('hex');
  fs.writeFileSync(path.join(cfg, 'bot_relay/native', `${expiredId}.json`), JSON.stringify({ ...envelope, id: expiredId }));
  fs.mkdirSync(path.join(cfg, 'bot_relay/replies'), { recursive: true });
  fs.writeFileSync(path.join(cfg, 'bot_relay/replies', `${expiredId}.json`), JSON.stringify({ id: expiredId, error: 'queued message expired', reason: 'queued_expired' }));
  primary = await start('default');
  await until(() => seen.some(s => s.user.includes('failed [reason: queued_expired]')), 15000);
  assert.equal(seen.filter(s => s.user.includes('failed [reason: queued_expired]')).length, 1);
  console.log('ok: durable expiry wakes the original chat after sender restart');
  const staleId = crypto.randomBytes(16).toString('hex');
  const stale = { ...envelope, id: staleId, created_at: 1 };
  fs.writeFileSync(path.join(cfg, 'bot_relay/native', `${staleId}.json`), JSON.stringify(stale));
  fs.writeFileSync(path.join(cfg, 'bot_relay/outbox', `${staleId}.json`), JSON.stringify(stale));
  await until(() => seen.some(s => s.user.includes('Queued message expired before the Desktop claimed it')), 15000);
  assert.ok(!fs.existsSync(path.join(cfg, 'bot_relay/outbox', `${staleId}.json`)));
  console.log('ok: unclaimed stale mail expires without a Desktop or Python waiter');
  const peerChat = await primary.call('session.create', { title: 'Peer check' });
  await primary.call('prompt.submit', { session_id: peerChat.session_id, text: 'NATIVE_PEER' });
  await until(() => peerRequests.some(r => r.method === 'POST'), 15000);
  const peerTurn = peerRequests.find(r => r.method === 'POST');
  assert.equal(peerTurn.url, '/p/researcher/api/sessions/hidden-chat/chat');
  assert.equal(peerTurn.body.message, 'Message from 🤖 factr (@factr): SLOW_DELIVERY native ping');
  assert.equal(peerTurn.body.author.is_bot, true);
  const goldenPeer = JSON.parse(fs.readFileSync(path.join(work, 'engine/crates/factr-gateway/tests/fixtures/peer_dm.json'), 'utf8'));
  const normalizePeer = (row: any) => {
    const url = new URL(row.url, 'http://peer'); url.searchParams.sort();
    const body = row.body ? { message: 'Message from 🤖 factr (@factr): hi' } : null;
    return { method: row.method, url: url.pathname + url.search, body };
  };
  assert.deepEqual(peerRequests.map(normalizePeer), goldenPeer.map(normalizePeer), 'existing Python peer HTTP golden');
  await until(() => seen.some(s => s.user.includes('PEER_ANSWER')), 15000);
  assert.equal(peerRequests.filter(r => r.method === 'POST').length, 1);
  console.log('ok: native registered peer REST contract and sender wake');
  fs.appendFileSync(path.join(cfg, 'config.yaml'), 'agent:\n  bot_mode_protocol: false\n');
  const disabledChat = await primary.call('session.create', { title: 'Protocol disabled' });
  await primary.call('prompt.submit', { session_id: disabledChat.session_id, text: 'PROTOCOL_DISABLED' });
  await until(() => seen.some(s => s.user === 'PROTOCOL_DISABLED'));
  assert.ok(!seen.find(s => s.user === 'PROTOCOL_DISABLED').body.tools.some(t => t.function.name === 'message_agent'));
  fs.writeFileSync(path.join(cfg, 'config.yaml'), fs.readFileSync(path.join(cfg, 'config.yaml'), 'utf8').replace('bot_mode_protocol: false', 'bot_mode_protocol: true'));
  console.log('ok: native protocol toggle removes message_agent');
  console.log("stage: retirement");
  const permit = await b.retirement("prepare");
  assert.equal(permit.idle, true, JSON.stringify(permit));
  const rejected = await b.raw("prompt.submit", {
    session_id: targetChat,
    text: "must reroute",
  });
  assert.equal(rejected.error?.data?.reason, "retiring");
  const refused = await b.raw("bot.chat.deliver", {
    text: "must reroute too",
    envelope_id: "rerouted",
  });
  assert.equal(refused.error?.data?.reason, "retiring");
  assert.equal((await b.retirement("commit", permit.token)).ok, true);
  await b.stop();
  pool.delete("b");
  releases.get("b")!();
  console.log("stage: restart");
  const restarted = await pooled("b");
  assert.equal((await restarted.call('bot.group.ensure', { profile: 'b', title: 'Group: runtime-room · thread' })).session_id, groupId);
  assert.equal(
    (await restarted.call("bot.chat.get", { profile: "b" })).canonical_session
      .id,
    targetChat,
  );
  assert.equal(
    (
      await delivery(restarted, {
        profile: "b",
        text: envelope.message,
        envelope_id: envelope.id,
      })
    ).reply,
    result.reply,
  );
  assert.equal(
    seen.filter((s) => s.user === envelope.message).length,
    1,
    "settled envelope is not delivered twice after restart",
  );
  const rerouted = await delivery(restarted, {
    text: "must reroute too",
    envelope_id: "rerouted",
  });
  assert.equal(rerouted.ok, true);
  restarted.activeTurn = true; // stale renderer state must not veto the engine's idle proof
  console.log("stage: pool churn");
  await pooled("c");
  await pooled("d");
  for (let round = 0; round < 7; round++) {
    const name = ["a", "b", "c", "d", "e"].find(name => !pool.has(name))!;
    const previousStops = stopped.length;
    const engine = await pooled(name);
    await engine.call("bot.chat.ensure", { profile: name });
    engine.lastActiveAt = Date.now();
    assert.ok(pool.size <= 3);
    assert.equal(stopped.length, previousStops + 1, "a cold open reclaims exactly one full pool slot");
  }
  assert.ok(
    stopped.length >= 5,
    "five-plus-bot churn evicts idle engines repeatedly",
  );
  for (const [name, engine] of pool) { await engine.stop(); releases.get(name)!() }
  pool.clear()
  seed("h")
  fs.unlinkSync(path.join(cfg, "profiles/h/profile.yaml"))
  fs.appendFileSync(path.join(cfg, "profiles/h/config.yaml"), `mcp_servers:\n  factr_teammates:\n    command: ${process.execPath}\n    args: ['-e', 'setInterval(() => {}, 1000)']\n    timeout_secs: 900\n`)
  const hung = await start("h")
  const began = Date.now()
  const refusedHandshake = await hung.raw("session.create", { title: "Bot Chat" })
  assert.equal(refusedHandshake.error?.data?.reason, "teammate_mcp_timeout")
  assert.ok(hung.events.some(e => e.type === "error" && e.payload?.reason === "teammate_mcp_timeout"))
  assert.ok(Date.now() - began < 23_000, "a legacy 900s MCP setting cannot hang initialization")
  await hung.stop()
  console.log(
    "PASS native queued ack, sender wake, reply idempotency, busy refusal, admission freeze, physical exit, restart persistence and 5-bot pool churn",
  );
} finally {
  peer.close();
  retirer.dispose();
  for (const child of children) child.kill("SIGTERM");
  await sleep(500);
  for (const child of children) child.kill("SIGKILL");
  model.closeAllConnections();
  model.close();
  console.log("Isolated runtime artifacts:", base);
}
