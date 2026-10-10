//! Drives real worker processes: state, recursion, confinement, limits.

use factr_learn::host::Refine;
use factr_learn::{LlmQuery, ReplHost};
use std::path::PathBuf;
use std::sync::Arc;

/// An env var these tests need, or a printed notice and an early return from the calling test
/// (so the default suite is green; CI sets them, and the tests then run).
macro_rules! env_or_skip {
    ($name:literal) => {
        match std::env::var($name) {
            Ok(value) => value,
            Err(_) => {
                eprintln!("skipped: {} is not set", $name);
                return;
            }
        }
    };
}

/// A REPL host on the staged Factr CPython.
macro_rules! host {
    () => {
        ReplHost::new(PathBuf::from(env_or_skip!("FACTR_REPL_PYTHON")))
    };
}

/// Generous worker memory cap for tests that are not about the cap.
const MEMORY_LIMIT: u64 = 2 << 30;
/// A cap the interpreter's own baseline fits under but a 400 MB allocation does not.
const TINY_LIMIT: u64 = 128 << 20;

fn upper() -> LlmQuery {
    Arc::new(|prompt: String| Box::pin(async move { Ok(prompt.to_uppercase()) }))
}

fn no_refine() -> Refine {
    Arc::new(|_op: String| Box::pin(async move { Ok(r#"{"scheduled":false}"#.to_string()) }))
}

fn workdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "learn-test-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/notes.txt"), "alpha\nbeta\ngamma\n").unwrap();
    dir
}

/// `$FACTR_HOME` for the skills these tests plant and read, or `None` (skip) when it is unset or is the
/// developer's real `~/.factr/engine`: a test must never create or remove files in the real skills directory.
fn skills_home() -> Option<PathBuf> {
    let configured = PathBuf::from(std::env::var_os("FACTR_HOME")?);
    if factr_base::real_home_guard::is_real_home_path(&configured) {
        eprintln!("skipped: FACTR_HOME is the real ~/.factr/engine");
        return None;
    }
    Some(configured)
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[tokio::test(flavor = "multi_thread")]
async fn variables_persist_between_runs() {
    let h = host!();
    let first = h
        .run(
            "s",
            "x = 41",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert!(first.fresh_state && first.error.is_none());
    let second = h
        .run(
            "s",
            "x + 1",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(second.value.as_deref(), Some("42"));
    assert!(!second.fresh_state);
    // Sessions are isolated from each other.
    let other = h
        .run(
            "other",
            "x",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert!(other.error.unwrap().contains("NameError"));
}

#[tokio::test(flavor = "multi_thread")]
async fn repl_cold_and_warm_latency_budgets_hold() {
    let h = host!();
    assert_eq!(
        h.live_workers().await,
        0,
        "constructing the REPL host must not start Python on turns that skip the tool"
    );
    let started = std::time::Instant::now();
    let cold = h
        .run(
            "latency-budget",
            "1 + 1",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    let cold_time = started.elapsed();
    assert!(cold.error.is_none());
    assert_eq!(h.live_workers().await, 1, "the first REPL call starts one worker");
    assert!(
        cold_time < std::time::Duration::from_millis(500),
        "REPL cold start was {cold_time:?}"
    );

    let mut samples = Vec::with_capacity(120);
    for _ in 0..120 {
        let started = std::time::Instant::now();
        let warm = h
            .run(
                "latency-budget",
                "1 + 1",
                None,
                upper(),
                no_refine(),
                factr_learn::host::ExtraHostFns::default(),
                MEMORY_LIMIT,
            )
            .await
            .unwrap();
        assert!(warm.error.is_none());
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let p95 = samples[(samples.len() * 95 / 100) - 1];
    println!(
        "REPL latency: cold={cold_time:?} warm p50={:?} p95={p95:?}",
        samples[samples.len() / 2]
    );
    assert!(
        p95 < std::time::Duration::from_millis(5),
        "warm REPL p95 was {p95:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stdlib_imports_and_python_package_skills_work() {
    // Skip (before planting anything) when there is no worker to import it.
    let h = host!();
    let Some(home) = skills_home() else { return };
    let skills = home.join("skills");
    let package = skills.join("parity_probe/src/parity_probe");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("__init__.py"),
        "def twice(value):\n    return value * 2\n",
    )
    .unwrap();
    let out = h.run(
        "imports",
        "import json, re\nimport parity_probe\nmatch = re.search(r'(\\d+)', 'value=21')\njson.dumps({'result': parity_probe.twice(int(match.group(1)))})",
        None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(),
        MEMORY_LIMIT,
    ).await.unwrap();
    assert_eq!(out.value.as_deref(), Some("'{\"result\": 42}'"));
    drop(h);
    std::fs::remove_dir_all(skills.join("parity_probe")).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn bundled_learn_skill_package_imports_and_calls_rust_host() {
    let Some(skills_root) = skills_home() else { return };
    let skills = skills_root.join("skills");
    let home = std::env::temp_dir().join(format!("learn-goal-skill-{}", rand_suffix()));
    std::fs::create_dir_all(&home).unwrap();
    let store = Arc::new(factr_learn::agent_loop::ControlStore::open(&home).unwrap());
    let store_for_host = store.clone();
    let h = host!();
    let extra = factr_learn::host::ExtraHostFns {
        goal: Arc::new(move |op| {
            let store = store_for_host.clone();
            Box::pin(async move {
                factr_learn::agent_loop_host::goal_host(&store, "goal-test", &op)
            })
        }),
        ..factr_learn::host::ExtraHostFns::default()
    };
    let out = h
        .run(
            "bundled-learn-skills",
            "import goal\ncreated = await goal.create('prove package bridge', token_budget=42)\ncurrent = await goal.get()\nprogressed = await goal.progress('probed the bridge', 'pass')\ncompleted = await goal.complete('ran the bridge probe: ok')\n(created['goal']['token_budget'], current['remaining_tokens'], completed['goal']['status'], completed['completion_budget_report']['token_budget'], progressed['goal']['attempt_log'][-1], completed['goal']['completion_verification'])",
            None,
            upper(),
            no_refine(),
            extra,
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(
        out.value.as_deref(),
        Some("(42, 42, 'done', 42, 'pass: probed the bridge', 'ran the bridge probe: ok')"),
        "{:?}",
        out.error
    );
    assert!(skills.join("learn-goal/SKILL.md").is_file());
    drop(store);
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn bundled_rlm_heartbeat_skill_uses_separate_persisted_records() {
    let home = std::env::temp_dir().join(format!("rlm-heartbeat-{}", rand_suffix()));
    std::fs::create_dir_all(&home).unwrap();
    let store = Arc::new(factr_learn::agent_loop::ControlStore::open(&home).unwrap());
    let user = factr_learn::agent_loop::Heartbeat::new("rlm-skills", "user reminder", 60);
    store.upsert_heartbeat(&user).unwrap();
    let store_for_host = store.clone();
    let extra = factr_learn::host::ExtraHostFns {
        heartbeat: Arc::new(move |op| {
            let store = store_for_host.clone();
            Box::pin(async move {
                factr_learn::agent_loop_host::heartbeat_host(&store, "rlm-skills", &op)
            })
        }),
        ..factr_learn::host::ExtraHostFns::default()
    };
    let out = host!()
        .run(
            "rlm-heartbeat-skill",
            "import rlm_heartbeat\ncreated = await rlm_heartbeat.create('check queue', interval='5m', label='queue', delivery_mode='follow_up')\nawait rlm_heartbeat.update(created['id'], status='pause')\nactive = await rlm_heartbeat.list()\nitems = await rlm_heartbeat.list(include_inactive=True)\nawait rlm_heartbeat.update(created['id'], status='resume')\n(items['heartbeats'][0]['status'], items['heartbeats'][0]['label'], len(active['heartbeats']))",
            None,
            upper(),
            no_refine(),
            extra,
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(
        out.value.as_deref(),
        Some("('paused', 'queue', 0)"),
        "{:?}",
        out.error
    );
    assert_eq!(
        store.user_heartbeat("rlm-skills").unwrap().unwrap().id,
        user.id
    );
    assert_eq!(store.list_heartbeats("rlm-skills").unwrap().len(), 2);
    drop(store);
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn llm_query_is_a_recursive_host_call() {
    let h = host!();
    let out = h.run("s", "parts = ['a', 'b']\nr = []\nfor p in parts:\n    r.append(await llm_query(p))\nprint(r)\n''.join(r)", None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT).await.unwrap();
    assert_eq!(out.value.as_deref(), Some("'AB'"));
    assert_eq!(out.stdout.trim(), "['A', 'B']");
    assert_eq!(out.host_calls, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_top_level_helper_call_without_await_is_awaited_and_a_passed_on_one_is_not() {
    let h = host!();
    let run = |code: &'static str| {
        h.run("await", code, None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT)
    };
    let out = run("s = llm_query('ab')\nllm_query('dropped')\nt: str = llm_query('cd')\ns + t").await.unwrap();
    assert_eq!((out.value.as_deref(), out.error.as_deref(), out.host_calls), (Some("'ABCD'"), None, 3));
    // A coroutine handed to something else stays a coroutine, and a rebound name is the cell's own.
    let out = run("import asyncio\nr = await asyncio.gather(llm_query('x'), llm_query('y'))\nr").await.unwrap();
    assert_eq!(out.value.as_deref(), Some("['X', 'Y']"));
    let out = run("load = lambda *a: 'mine'\nv = load('p')\nv").await.unwrap();
    assert_eq!(out.value.as_deref(), Some("'mine'"));
}

#[tokio::test(flavor = "multi_thread")]
async fn learn_skill_host_request_bridges_goal_and_refine() {
    let h = host!();
    let extra = factr_learn::host::ExtraHostFns {
        goal: Arc::new(|op| {
            Box::pin(async move {
                let op: serde_json::Value = serde_json::from_str(&op).unwrap();
                match op["op"].as_str().unwrap() {
                    "get" => Ok(r#"{"goal":"ship parity"}"#.to_string()),
                    "create" => Ok(format!(r#"{{"created":{}}}"#, op["text"])),
                    other => anyhow::bail!("unexpected goal operation: {other}"),
                }
            })
        }),
        websearch: Arc::new(|op| {
            Box::pin(async move {
                let op: serde_json::Value = serde_json::from_str(&op).unwrap();
                anyhow::ensure!(op["query"] == "local", "query was not forwarded");
                anyhow::ensure!(op["num_results"] == 2, "result limit was not forwarded");
                Ok("safe local results".to_string())
            })
        }),
        agent_message: Arc::new(|op| {
            Box::pin(async move {
                let op: serde_json::Value = serde_json::from_str(&op)?;
                let request = op["host_request"].as_str().unwrap();
                let payload = &op["payload"];
                match request {
                    "agent_observe.list" => Ok(
                        r#"{"agents":[{"session_id":"child","relationship":"child"}]}"#.to_string(),
                    ),
                    "agent_observe.get" => Ok(r#"{"agent":{"session_id":"child"}}"#.to_string()),
                    "agent_observe.recent" => {
                        Ok(r#"{"messages":[{"role":"assistant","content":"pong"}]}"#.to_string())
                    }
                    "agent_message.send" => {
                        anyhow::ensure!(payload["receiver_role"] == "child", "role not forwarded");
                        anyhow::ensure!(payload["receiver_name"] == "worker", "name not forwarded");
                        Ok(r#"{"receipts":[{"deliveryStatus":"sent"}]}"#.to_string())
                    }
                    other => anyhow::bail!("unexpected factr-learn operation: {other}"),
                }
            })
        }),
        spawn_subagent: Arc::new(|op| {
            Box::pin(async move {
                let op: serde_json::Value = serde_json::from_str(&op)?;
                if op["action"] == "await" {
                    Ok(
                        r#"{"session_id":"child-id","status":"completed","result":"pong"}"#
                            .to_string(),
                    )
                } else {
                    anyhow::ensure!(op["label"] == "worker", "subagent label not forwarded");
                    Ok(r#"{"session_id":"child-id","status":"spawned"}"#.to_string())
                }
            })
        }),
        compact: Arc::new(|op| {
            Box::pin(async move {
                let op: serde_json::Value = serde_json::from_str(&op)?;
                match op["op"].as_str().unwrap() {
                    "status" => Ok(r#"{"scheduled":true}"#.to_string()),
                    "run" => {
                        anyhow::ensure!(
                            op["instructions"] == "retain the decision",
                            "focus not bridged"
                        );
                        Ok(r#"{"scheduled":true,"phase":"end_of_turn"}"#.to_string())
                    }
                    other => anyhow::bail!("unexpected compact operation: {other}"),
                }
            })
        }),
        ..factr_learn::host::ExtraHostFns::default()
    };
    let out = h.run(
        "learn-api",
        "import rlm, agent_observe, agent_message, compact\ngoal = await rlm.host_request('goal.get')\ncreated = await rlm.host_request('goal.create', {'objective': 'ship parity'})\nrefine = await rlm.host_request('refine.status')\nsearch = await rlm.host_request('websearch.run', {'query': 'local', 'num_results': 2})\nroster = await agent_observe.list_agents()\nagent = await agent_observe.get_agent('child')\nrecent = await agent_observe.recent_messages('child', limit=1, max_chars=100)\nreceipt = await agent_message.send('hi', receiver_role='child', receiver_name='worker')\nchild = await spawn_subagent('reply pong', name='worker')\nfinished = await await_subagent(child['session_id'])\ncs = await compact.status()\ncr = await compact.run('retain the decision')\n(goal['goal'], created['created'], refine['scheduled'], search['results'], roster['agents'][0]['relationship'], agent['agent']['session_id'], recent['messages'][0]['content'], receipt['receipts'][0]['deliveryStatus'], finished['result'], cs['scheduled'], cr['phase'])",
        None,
        upper(),
        no_refine(),
        extra,
        MEMORY_LIMIT,
    ).await.unwrap();
    assert_eq!(
        out.value.as_deref(),
        Some(
            "('ship parity', 'ship parity', False, 'safe local results', 'child', 'child', 'pong', 'sent', 'pong', True, 'end_of_turn')"
        )
    );
    assert_eq!(out.host_calls, 12);
}

#[tokio::test(flavor = "multi_thread")]
async fn load_reads_workspace_files_into_variables() {
    let h = host!();
    let dir = workdir();
    let out = h
        .run(
            "s",
            "t = await load('src/notes.txt')\nlen(t.splitlines())",
            Some(&dir),
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(out.value.as_deref(), Some("3"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_worker_starts_in_the_project_dir_with_relative_file_access() {
    let h = host!();
    let dir = workdir();
    let code = "import os, glob\n\
        here = os.path.realpath(os.getcwd())\n\
        names = sorted(os.listdir('.'))\n\
        txt = open('src/notes.txt').read()\n\
        open('out.txt', 'w').write('written')\n\
        (here, names, len(txt.splitlines()), glob.glob('src/*.txt'), open('out.txt').read())";
    let out = h
        .run(
            "s",
            code,
            Some(&dir),
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert!(out.error.is_none(), "{:?}", out.error);
    let value = out.value.unwrap_or_default();
    let real = std::fs::canonicalize(&dir).unwrap();
    assert!(value.contains(real.to_str().unwrap()), "{value}");
    assert!(value.contains("'src'") && value.contains("3") && value.contains("src/notes.txt"), "{value}");
    assert!(value.contains("'written'"), "{value}");
    assert_eq!(std::fs::read_to_string(dir.join("out.txt")).unwrap(), "written");
    // Still confined: the parent directory and the home directory are not readable or writable.
    let outside = dir.parent().unwrap().join(format!("learn-parent-secret-{}", rand_suffix()));
    std::fs::write(&outside, "secret").unwrap();
    let probe = format!(
        "import os\nres = []\nfor f, mode in [('../{}', 'r'), ('../w.txt', 'w'), (os.path.expanduser('~/.zshrc'), 'r'), ('/etc/hosts', 'r')]:\n    try:\n        open(f, mode); res.append('OPEN')\n    except Exception as e:\n        res.append(type(e).__name__)\nres",
        outside.file_name().unwrap().to_str().unwrap()
    );
    let denied = h
        .run("s", &probe, Some(&dir), upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    let value = denied.value.unwrap_or_default();
    assert!(!value.contains("OPEN"), "{value} {:?}", denied.error);
    std::fs::remove_file(outside).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn load_is_confined_to_the_working_directory() {
    let h = host!();
    let dir = workdir();
    let outside = std::env::temp_dir().join(format!("learn-outside-{}", rand_suffix()));
    std::fs::write(&outside, "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, dir.join("link.txt")).unwrap();
    for path in ["../x", "/etc/passwd", "link.txt", outside.to_str().unwrap()] {
        let code = format!("load({path:?})");
        let out = h
            .run(
                "s",
                &code,
                Some(&dir),
                upper(),
                no_refine(),
                factr_learn::host::ExtraHostFns::default(),
                MEMORY_LIMIT,
            )
            .await
            .unwrap();
        let err = out.error.unwrap_or_default();
        assert!(
            err.contains("outside the working directory") || err.contains("not found"),
            "{path}: {err}"
        );
    }
    let out = h
        .run(
            "s",
            "load('src/notes.txt')",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert!(out.error.unwrap().contains("working directory"));
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_file(outside).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sandbox_has_no_host_access() {
    let h = host!();
    for (code, expect) in [
        ("open('/etc/passwd').read()", "PermissionError"),
        ("import os\nos.listdir('/Users')", "PermissionError"),
        (
            "import socket\nsocket.create_connection(('127.0.0.1', 9), .1)",
            "PermissionError",
        ),
        (
            "open('/private/tmp/factr-repl-outside-write', 'w').write('x')",
            "PermissionError",
        ),
    ] {
        let out = h
            .run(
                "s",
                code,
                None,
                upper(),
                no_refine(),
                factr_learn::host::ExtraHostFns::default(),
                MEMORY_LIMIT,
            )
            .await
            .unwrap();
        let err = out.error.unwrap_or_default();
        assert!(err.contains(expect), "{code}: {err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn host_call_budget_is_enforced() {
    let h = host!();
    let out = h
        .run(
            "s",
            "for i in range(40):\n    await llm_query(str(i))",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert!(out.error.unwrap().contains("budget"));
    assert_eq!(out.host_calls, 16);
}

#[tokio::test(flavor = "multi_thread")]
async fn runaway_code_hits_limits_and_the_session_survives() {
    let h = host!();
    h.run(
        "s",
        "keep = 7",
        None,
        upper(),
        no_refine(),
        factr_learn::host::ExtraHostFns::default(),
        MEMORY_LIMIT,
    )
    .await
    .unwrap();
    let out = h
        .run(
            "s",
            "def f(n):\n    return f(n + 1)\nf(0)",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert!(out.error.unwrap().contains("RecursionError"));
    let out = h
        .run(
            "s",
            "keep",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(
        out.value.as_deref(),
        Some("7"),
        "state survives a caught error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_blowup_is_contained_in_the_worker() {
    let h = host!();
    let started = std::time::Instant::now();
    let result = h
        .run(
            "s",
            "z = b'x' * (400 * 1024 * 1024)\nimport time\ntime.sleep(3)",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            TINY_LIMIT,
        )
        .await;
    match result {
        Ok(out) => assert!(
            out.error.unwrap_or_default().contains("MemoryError"),
            "expected MemoryError"
        ),
        Err(err) => assert!(format!("{err:#}").contains("stream or chunk"), "{err:#}"),
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "RSS watchdog did not stop the worker promptly"
    );
    // The host is still usable afterwards.
    let out = h
        .run(
            "s",
            "1 + 1",
            None,
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(out.value.as_deref(), Some("2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_query_and_sliced_load_work_in_the_sandbox() {
    let h = host!();
    let dir = workdir();
    let out = h
        .run(
            "s",
            "r = await llm_query_batch(['a', 'b'])\ns = await load('src/notes.txt', 6, 4)\n(r, s)",
            Some(&dir),
            upper(),
            no_refine(),
            factr_learn::host::ExtraHostFns::default(),
            MEMORY_LIMIT,
        )
        .await
        .unwrap();
    assert_eq!(out.value.as_deref(), Some("(['A', 'B'], 'beta')"));
    assert_eq!(out.host_calls, 2, "a batch counts as one host call");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_over_long_cell_is_interrupted_and_keeps_its_variables() {
    let h = host!();
    h.run("s", "kept = [1, 2, 3]", None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    let out = h
        .run("s", "while True:\n    pass", None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    let error = out.error.expect("the interrupted cell reports an error");
    assert!(error.contains("interrupted") && error.contains("KeyboardInterrupt"), "{error}");
    let out = h
        .run("s", "kept", None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    assert_eq!(out.value.as_deref(), Some("[1, 2, 3]"));
    assert!(!out.fresh_state, "the worker was not restarted");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exception_reports_the_traceback_from_the_cell() {
    let h = host!();
    let out = h
        .run("s", "def f():\n    return 1 / 0\nf()", None, upper(), no_refine(), factr_learn::host::ExtraHostFns::default(), MEMORY_LIMIT)
        .await
        .unwrap();
    let error = out.error.unwrap();
    assert!(error.starts_with("Traceback") && error.contains("in f") && error.ends_with("ZeroDivisionError: division by zero"), "{error}");
    assert!(!error.contains("python_worker") && !error.contains("asyncio"), "{error}");
}

/// Real worker and host: `classify` takes the default (json) path with usage metering, the log lands in the
/// file `FACTR_CLASSIFY_LOG` names (hashes and counts only), and writing it spends no host-call budget.
#[tokio::test(flavor = "multi_thread")]
async fn classify_logs_per_chunk_without_spending_host_calls_or_leaking_text() {
    let h = host!();
    // A directory: each engine process writes its own `classify-<pid>.jsonl` in it.
    let dir = std::env::temp_dir().join(format!("classify-log-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join(format!("classify-{}.jsonl", std::process::id()));
    // SAFETY: this test is the only one in the binary that reads or sets this variable.
    unsafe { std::env::set_var("FACTR_CLASSIFY_LOG", &dir) };
    let meta: factr_learn::host::LlmQueryMeta = Arc::new(|prompt: String| {
        Box::pin(async move {
            let rows: Vec<&str> = prompt.split("Items:\n").nth(1).unwrap().lines().collect();
            let text = rows.iter().map(|l| l.split_once(". ").unwrap().0).map(|id| format!("\"{id}\":\"no\"")).collect::<Vec<_>>().join(",");
            let text = format!("{{{text}}}");
            Ok(factr_learn::host::SubReply { text, input_tokens: Some(500), output_tokens: Some(40), cached_tokens: Some(0), reasoning_tokens: None, latency_ms: 12, effort: Some("low".into()), ..Default::default() })
        })
    });
    let extra = factr_learn::host::ExtraHostFns { llm_query_meta: Some(meta), sub_effort: "low".into(), ..Default::default() };
    let out = h
        .run("s", "r = await classify(['secret one', 'secret two', 'secret one'], ['yes', 'no'])\nr", None, upper(), no_refine(), extra, MEMORY_LIMIT)
        .await
        .unwrap();
    unsafe { std::env::remove_var("FACTR_CLASSIFY_LOG") };
    assert_eq!(out.error, None);
    assert_eq!(out.value.as_deref(), Some("['no', 'no', 'no']"));
    assert_eq!(out.host_calls, 1, "one batch call; the log frames are not counted");
    let text = std::fs::read_to_string(&log).unwrap();
    #[cfg(unix)]
    assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&log).unwrap().permissions()) & 0o777, 0o600);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!text.contains("secret"), "{text}");
    let chunk: serde_json::Value = text.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|r| r["type"] == "call").unwrap();
    assert_eq!((chunk["chunk_size"].as_i64(), chunk["input_tokens"].as_i64(), chunk["effort"].as_str()), (Some(3), Some(500), Some("low")), "defaults: no dedupe");
    assert_eq!((chunk["format"].as_str(), chunk["chunk_items"].as_i64(), chunk["chunk_chars"].as_i64(), chunk["chunk_source"].as_str()), (Some("json"), Some(80), Some(48000), Some("default-80")));
    // The engine stamps every row with the session and its cell, and scopes the worker-local ids by them.
    assert_eq!(chunk["session"], "s");
    assert!(chunk["cell"].as_u64().is_some_and(|c| c >= 1), "{chunk}");
    assert!(chunk["call_id"].as_str().unwrap().starts_with(&format!("s/{}/", chunk["cell"])), "{chunk}");
    assert!(text.lines().any(|l| l.contains("\"type\":\"job\"") && l.contains("\"status\":\"ok\"")), "{text}");
}
