//! Worker protocol against a scripted fake host (plain `python3`, no sandbox).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const WORKER: &str = include_str!("../src/python_worker.py");

/// Runs `code` in a fresh worker; `host` answers each call frame `(fn, args)`
/// with a reply value or error. Returns the `done` frame and the calls seen.
fn run(code: &str, host: impl Fn(&str, &[String]) -> Result<String, String>) -> Option<(Value, Vec<String>)> {
    let dir = std::env::temp_dir().join(format!("worker-fake-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).ok()?;
    let mut child = Command::new("python3")
        .args(["-I", "-S", "-u", "-c", WORKER])
        .arg(&dir)
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).ok()?;
    writeln!(stdin, "{}", json!({"op": "run", "code": code})).unwrap();
    let mut calls = Vec::new();
    loop {
        line.clear();
        out.read_line(&mut line).unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        match msg["op"].as_str() {
            Some("done") => {
                let _ = child.kill();
                return Some((msg, calls));
            }
            Some("call") => {
                let name = msg["fn"].as_str().unwrap().to_string();
                let args: Vec<String> = msg["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
                calls.push(name.clone());
                let reply = match host(&name, &args) {
                    Ok(v) => json!({"op": "reply", "value": v}),
                    Err(e) => json!({"op": "reply", "error": e}),
                };
                writeln!(stdin, "{reply}").unwrap();
            }
            _ => panic!("unexpected frame {line}"),
        }
    }
}

#[test]
fn batch_is_one_host_call_with_ordered_replies_and_error_strings() {
    let Some((done, calls)) = run(
        "r = await llm_query_batch(['a', 'bad', 'c'])\nr",
        |name, args| {
            assert_eq!(name, "llm_query_batch");
            let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
            Ok(json!(prompts.iter().map(|p| if p == "bad" { "Error: boom".into() } else { p.to_uppercase() }).collect::<Vec<String>>()).to_string())
        },
    ) else {
        eprintln!("skipped: no python3");
        return;
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(done["host_calls"], 1);
    assert_eq!(done["value"], "['A', 'Error: boom', 'C']");
}

#[test]
fn batch_rejects_a_bare_string() {
    let Some((done, calls)) = run("await llm_query_batch('abc')", |_, _| Ok("[]".into())) else { return };
    assert!(calls.is_empty());
    assert!(done["error"].as_str().unwrap().contains("list of strings"));
}

#[test]
fn load_reads_only_the_requested_slice() {
    let dir = std::env::temp_dir().join(format!("worker-slice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("big.txt");
    std::fs::write(&file, "0123456789abcdef").unwrap();
    let info = json!({"path": file, "size": 16}).to_string();
    let host = move |name: &str, _: &[String]| {
        assert_eq!(name, "load_path");
        Ok(info.clone())
    };
    let Some((done, _)) = run("(await load('big.txt', 4, 6), await load('big.txt', 12), await load('big.txt', 99), len(await load('big.txt')))", host) else { return };
    assert_eq!(done["value"], "('456789', 'cdef', '', 16)");
}

/// Fake sub-model for `classify`: replies per numbered line `N. text`; `mode(call, prompt_index, ids)` may
/// return a raw reply instead of the correct JSON object. Text starting with "a" is "alpha ray", else "beta".
fn labelled(prompts_json: &str, mode: &dyn Fn(usize, usize, &[usize]) -> Option<String>, call: usize) -> String {
    let prompts: Vec<String> = serde_json::from_str(prompts_json).unwrap();
    let replies: Vec<String> = prompts
        .iter()
        .enumerate()
        .map(|(n, p)| {
            let rows: Vec<(usize, &str)> = p.split("Items:\n").nth(1).unwrap().lines().map(|l| {
                let (id, text) = l.split_once(". ").unwrap();
                (id.parse().unwrap(), text)
            }).collect();
            let ids: Vec<usize> = rows.iter().map(|r| r.0).collect();
            mode(call, n, &ids).unwrap_or_else(|| {
                let map: serde_json::Map<String, Value> = rows.iter().map(|(i, t)| (i.to_string(), json!(if t.starts_with('a') { "alpha ray" } else { "beta" }))).collect();
                Value::Object(map).to_string()
            })
        })
        .collect();
    json!(replies).to_string()
}

fn classify_run(code: &str, mode: impl Fn(usize, usize, &[usize]) -> Option<String>) -> Option<(Value, Vec<String>)> {
    let call = std::cell::Cell::new(0usize);
    run(code, |name, args| {
        assert_eq!(name, "llm_query_batch");
        call.set(call.get() + 1);
        Ok(labelled(&args[0], &mode, call.get()))
    })
}

const CLASSIFY_CODE: &str = "r = await classify(['apple', 'bean', 'avocado', 'corn'], ['alpha ray', 'beta'])\nr";

#[test]
fn classify_labels_every_item_in_one_batch_call() {
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, |_, _, _| None) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(calls.len(), 1);
}

#[test]
fn classify_requeries_missing_ids_and_keeps_the_valid_ones() {
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, |call, _, _| (call == 1).then(|| r#"{"1": "alpha ray", "3": "alpha ray"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(calls.len(), 2);
}

#[test]
fn classify_rejects_extra_and_repeated_ids_and_unknown_labels_then_gives_up_loudly() {
    for bad in [
        r#"{"1": "alpha ray", "2": "beta", "3": "alpha ray", "4": "beta", "9": "beta"}"#,
        r#"{"1": "alpha ray", "1": "beta", "2": "beta", "3": "alpha ray", "4": "beta"}"#,
        r#"{"1": "gamma", "2": "gamma", "3": "gamma", "4": "gamma"}"#,
        "not json at all",
    ] {
        let Some((done, calls)) = classify_run(CLASSIFY_CODE, move |_, _, _| Some(bad.to_string())) else { return };
        assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{bad}: {done}");
        assert_eq!(calls.len(), 3, "first ask plus two retries: {bad}");
    }
}

#[test]
fn classify_normalises_case_and_label_fragments() {
    let reply = r#"```json
{"1": "ALPHA RAY.", "2": "Beta", "3": "alpha", "4": " beta "}
```"#;
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, move |_, _, _| Some(reply.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(calls.len(), 1);
}

#[test]
fn classify_ambiguous_fragment_is_not_accepted() {
    let code = "await classify(['apple'], ['red ant', 'red bee'])";
    let Some((done, _)) = classify_run(code, |_, _, _| Some(r#"{"1": "red"}"#.to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{done}");
}

#[test]
fn classify_votes_reask_only_disagreeing_items() {
    // Pass 0 and pass 1 are one batch call; they disagree only on item 2. The tie-break asks that item alone.
    let code = "r = await classify(['apple', 'bean', 'corn'], ['alpha ray', 'beta'], votes=3)\nr";
    let sizes = std::cell::RefCell::new(Vec::new());
    let calls = std::cell::Cell::new(0usize);
    let Some((done, seen)) = run(code, |_, args| {
        calls.set(calls.get() + 1);
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        sizes.borrow_mut().push(prompts.iter().map(|p| p.split("Items:\n").nth(1).unwrap().lines().count()).collect::<Vec<_>>());
        let mode = |call: usize, n: usize, _: &[usize]| (call == 1 && n == 1).then(|| r#"{"1": "alpha ray", "2": "alpha ray", "3": "beta"}"#.to_string());
        Ok(labelled(&args[0], &mode, calls.get()))
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta']", "{done}");
    assert_eq!(seen.len(), 2);
    assert_eq!(*sizes.borrow(), vec![vec![3, 3], vec![1]]);
}

#[test]
fn classify_empty_input_makes_no_call() {
    let Some((done, calls)) = run("await classify([], ['a', 'b'])", |_, _| Ok("[]".into())) else { return };
    assert!(calls.is_empty());
    assert_eq!(done["value"], "[]");
}

#[test]
fn classify_chunks_many_items_within_one_wave() {
    let code = "r = await classify([str(i) for i in range(100)], ['alpha ray', 'beta'])\nlen(r)";
    let Some((done, calls)) = classify_run(code, |_, _, _| None) else { return };
    assert_eq!(done["value"], "100", "{done}");
    assert_eq!(calls.len(), 1, "3 chunks, one batch call");
}
