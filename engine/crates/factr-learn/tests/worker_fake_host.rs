//! Worker protocol against a scripted fake host (plain `python3`, no sandbox).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const WORKER: &str = include_str!("../src/python_worker.py");

/// Runs `code` in a fresh worker; `host` answers each call frame `(fn, args)`
/// with a reply value or error. Returns the `done` frame and the calls seen.
fn run(code: &str, host: impl Fn(&str, &[String]) -> Result<String, String>) -> Option<(Value, Vec<String>)> {
    run_cfg(code, json!({}), host)
}

/// [`run`] with the `classify` settings the engine would send along with the cell.
/// Most tests below exercise the compact `codes` format and dedupe, now opt-in, so those two (and the 40/24000 chunk size, a user setting here) are switched on here
/// unless the test says otherwise; `run_raw` sends the settings untouched (the shipped defaults).
fn run_cfg(code: &str, mut cfg: Value, host: impl Fn(&str, &[String]) -> Result<String, String>) -> Option<(Value, Vec<String>)> {
    for (k, v) in [("format", json!("codes")), ("dedupe", json!(true)), ("chunk_items", json!(40)), ("chunk_chars", json!(24000))] {
        if cfg.get(k).is_none() {
            cfg[k] = v;
        }
    }
    run_raw(code, cfg, host)
}

fn run_raw(code: &str, cfg: Value, host: impl Fn(&str, &[String]) -> Result<String, String>) -> Option<(Value, Vec<String>)> {
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
    writeln!(stdin, "{}", json!({"op": "run", "code": code, "cfg": cfg})).unwrap();
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

/// Fake sub-model for `classify`. It reads the numbered lines of each prompt and answers by the format the
/// prompt asks for (`id:letter` lines or a JSON object of label strings); `mode(call, prompt_index, ids)` may
/// return a raw reply instead. Text starting with "a" is "alpha ray" (letter A), else "beta" (letter B).
fn rows_of(prompt: &str) -> Vec<(usize, String)> {
    prompt.split("Items:\n").nth(1).unwrap().lines().map(|l| {
        let (id, text) = l.split_once(". ").unwrap();
        (id.parse().unwrap(), text.to_string())
    }).collect()
}

fn labelled(prompts_json: &str, mode: &dyn Fn(usize, usize, &[usize]) -> Option<String>, call: usize) -> Vec<String> {
    let prompts: Vec<String> = serde_json::from_str(prompts_json).unwrap();
    prompts
        .iter()
        .enumerate()
        .map(|(n, p)| {
            let rows = rows_of(p);
            let ids: Vec<usize> = rows.iter().map(|r| r.0).collect();
            mode(call, n, &ids).unwrap_or_else(|| {
                if p.contains("\nCodes: ") {
                    rows.iter().map(|(i, t)| format!("{i}:{}", if t.starts_with('a') { 'A' } else { 'B' })).collect::<Vec<_>>().join("\n")
                } else {
                    let map: serde_json::Map<String, Value> = rows.iter().map(|(i, t)| (i.to_string(), json!(if t.starts_with('a') { "alpha ray" } else { "beta" }))).collect();
                    Value::Object(map).to_string()
                }
            })
        })
        .collect()
}

/// The host's answer to a sub-query batch: plain strings for `llm_query_batch`, objects with usage for
/// `llm_query_batch_meta` (a reply starting with "Error:" becomes a transport error).
fn batch_reply(name: &str, replies: Vec<String>) -> String {
    if name == "llm_query_batch" {
        return json!(replies).to_string();
    }
    let rows: Vec<Value> = replies.iter().map(|r| match r.strip_prefix("Error:") {
        Some(e) => json!({"t": "", "e": format!("Error:{e}"), "i": null, "o": null, "c": null, "r": null, "ms": null, "f": null}),
        None => json!({"t": r, "e": null, "i": 120, "o": 30, "c": 64, "r": null, "ms": 250, "s": 5, "f": "low"}),
    }).collect();
    json!(rows).to_string()
}

const BATCH: [&str; 2] = ["llm_query_batch", "llm_query_batch_meta"];

fn classify_cfg_run(code: &str, cfg: Value, mode: impl Fn(usize, usize, &[usize]) -> Option<String>) -> Option<(Value, Vec<String>)> {
    let call = std::cell::Cell::new(0usize);
    run_cfg(code, cfg, |name, args| {
        if name == "classify_log" || name == "sleep_ms" {
            return Ok(String::new());
        }
        assert!(BATCH.contains(&name), "{name}");
        call.set(call.get() + 1);
        Ok(batch_reply(name, labelled(&args[0], &mode, call.get())))
    })
}

fn classify_run(code: &str, mode: impl Fn(usize, usize, &[usize]) -> Option<String>) -> Option<(Value, Vec<String>)> {
    classify_cfg_run(code, json!({}), mode)
}

fn batches(calls: &[String]) -> usize {
    calls.iter().filter(|c| BATCH.contains(&c.as_str())).count()
}

/// Runs `code` with a fake sub-model; returns the done frame, the size of every prompt's item list per batch
/// call, and the backoff waits the worker asked the engine for (seconds).
fn traced(code: &str, cfg: Value, mode: impl Fn(usize, usize, &[usize]) -> Option<String>) -> Option<(Value, Vec<Vec<usize>>, Vec<f64>)> {
    let sizes = std::cell::RefCell::new(Vec::new());
    let naps = std::cell::RefCell::new(Vec::new());
    let call = std::cell::Cell::new(0usize);
    let (done, _) = run_cfg(code, cfg, |name, args| {
        match name {
            "classify_log" => return Ok(String::new()),
            "sleep_ms" => {
                naps.borrow_mut().push(args[0].parse::<f64>().unwrap() / 1000.0);
                return Ok(String::new());
            }
            _ => {}
        }
        call.set(call.get() + 1);
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        sizes.borrow_mut().push(prompts.iter().map(|p| rows_of(p).len()).collect::<Vec<_>>());
        Ok(batch_reply(name, labelled(&args[0], &mode, call.get())))
    })?;
    let (sizes, naps) = (sizes.into_inner(), naps.into_inner());
    Some((done, sizes, naps))
}

const CLASSIFY_CODE: &str = "r = await classify(['apple', 'bean', 'avocado', 'corn'], ['alpha ray', 'beta'])\nr";

#[test]
fn classify_labels_every_item_in_one_batch_call() {
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, |_, _, _| None) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(calls, ["llm_query_batch_meta"]);
}

#[test]
fn classify_asks_for_id_letter_lines_with_a_quoted_stable_code_table() {
    let seen = std::cell::RefCell::new(Vec::new());
    let Some((done, _)) = run("r = await classify(['apple', 'bean'], ['alpha ray', 'beta'], guidance='Be brief.', votes=2)\nr", |_, args| {
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        seen.borrow_mut().extend(prompts.clone());
        Ok(batch_reply("llm_query_batch_meta", labelled(&args[0], &|_, _, _| None, 1)))
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta']", "{done}");
    let seen = seen.borrow();
    assert_eq!(seen.len(), 2, "votes=2: two passes");
    // The letters belong to the labels, whatever order a pass lists them in; ids are numbers, codes are letters.
    assert!(seen[0].contains("Codes: A=\"alpha ray\" | B=\"beta\"\n"), "{}", seen[0]);
    assert!(seen[1].contains("Codes: B=\"beta\" | A=\"alpha ray\"\n"), "{}", seen[1]);
    for p in seen.iter() {
        assert!(p.contains("Be brief.\n") && p.contains("\"<id>:<letter>\"") && p.contains("Items:\n1. apple\n2. bean"), "{p}");
        assert!(!p.contains("JSON"), "{p}");
    }
}

#[test]
fn awkward_labels_are_quoted_and_too_many_or_unsafe_labels_use_the_json_format() {
    let seen = std::cell::RefCell::new(Vec::new());
    let host = |code: &str| {
        seen.borrow_mut().clear();
        run(code, |_, args| {
            let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
            seen.borrow_mut().extend(prompts);
            Ok(batch_reply("llm_query_batch_meta", labelled(&args[0], &|_, _, _| None, 1)))
        })
    };
    // Separators and quotes inside a label cannot corrupt the table.
    let Some((done, _)) = host(r#"await classify(['apple'], ['a | b=c: "d"', 'beta'])"#) else { return };
    assert_eq!(done["value"], r#"['a | b=c: "d"']"#, "{done}");
    assert!(seen.borrow()[0].contains(r#"Codes: A="a | b=c: \"d\"" | B="beta""#), "{}", seen.borrow()[0]);
    // A label with a newline (or any non-printable character) switches this call to the JSON format.
    let Some((done, _)) = host("await classify(['apple'], ['alpha\\nray', 'beta'])") else { return };
    assert!(done["error"].is_null(), "{done}");
    assert!(seen.borrow()[0].contains("Allowed labels") && !seen.borrow()[0].contains("\nCodes: "), "{}", seen.borrow()[0]);
    // More labels than letters: JSON as well.
    let code = "labels = ['alpha ray', 'beta'] + [f'x{i}' for i in range(26)]\nawait classify(['apple', 'bean'], labels)";
    let Some((done, _)) = host(code) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta']", "{done}");
    assert!(seen.borrow()[0].contains("Allowed labels"), "{}", seen.borrow()[0]);
    // Exactly 26 labels still use letters.
    let code = "labels = ['alpha ray', 'beta'] + [f'x{i}' for i in range(24)]\nawait classify(['apple'], labels)";
    let Some((_, _)) = host(code) else { return };
    assert!(seen.borrow()[0].contains("Z=\"x23\""), "{}", seen.borrow()[0]);
}

#[test]
fn an_empty_label_list_is_a_value_error_not_a_modulo_by_zero() {
    let Some((done, calls)) = run("await classify(['apple'], [])", |_, _| Ok("[]".into())) else { return };
    assert!(done["error"].as_str().unwrap().contains("ValueError") && done["error"].as_str().unwrap().contains("non-empty"), "{done}");
    assert!(calls.is_empty());
}

#[test]
fn classify_json_format_switch_and_legacy_send_the_0_0_3_prompt_byte_for_byte() {
    let expected = "Classify every numbered item with exactly one label from the allowed list. Judge each item by its own text only.\nAllowed labels (use these exact strings): [\"alpha ray\", \"beta\"]\nBe brief.\nReply with ONLY a JSON object that maps every id below to one allowed label, each id exactly once, like {\"<id>\": \"<label>\"}. No other text.\nItems:\n1. apple\n2. bean";
    // Legacy asks through the metered batch call too (for the log); the prompts are the 0.0.3 bytes.
    for (cfg, name) in [(json!({"format": "json"}), "llm_query_batch_meta"), (json!({"legacy": true}), "llm_query_batch_meta")] {
        let seen = std::cell::RefCell::new(Vec::new());
        let Some((done, calls)) = run_cfg("r = await classify(['apple', 'bean'], ['alpha ray', 'beta'], guidance='Be brief.')\nr", cfg.clone(), |n, args| {
            let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
            seen.borrow_mut().extend(prompts);
            Ok(batch_reply(n, labelled(&args[0], &|_, _, _| None, 1)))
        }) else { return };
        assert_eq!(done["value"], "['alpha ray', 'beta']", "{done}");
        assert_eq!(calls, [name]);
        assert_eq!(seen.borrow().as_slice(), [expected], "{cfg}");
    }
}

#[test]
fn json_mode_never_takes_a_number_and_codes_mode_never_takes_one_in_a_json_reply() {
    // The model was shown label strings, so 0 or 1 means nothing it could have chosen.
    for cfg in [json!({"format": "json"}), json!({})] {
        let Some((done, _)) = classify_cfg_run(CLASSIFY_CODE, cfg.clone(), |_, _, _| Some(r#"{"1": 0, "2": 1, "3": 0, "4": 1}"#.to_string())) else { return };
        assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{cfg}: {done}");
    }
    // A single letter in a codes-mode JSON reply is the letter it was shown; in json mode it is not a label.
    let Some((done, _)) = classify_run(CLASSIFY_CODE, |_, _, _| Some(r#"{"1": "A", "2": "B", "3": "a", "4": "b"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    let Some((done, _)) = classify_cfg_run(CLASSIFY_CODE, json!({"format": "json"}), |_, _, _| Some(r#"{"1": "A", "2": "B", "3": "A", "4": "B"}"#.to_string())) else { return };
    assert!(done["error"].is_string(), "{done}");
}

#[test]
fn legacy_keeps_the_0_0_3_matching_dedupe_and_halving() {
    // Fragment matching and a repeated item asked twice: exactly the old behaviour.
    let reply = r#"{"1": "alpha", "2": "beta", "3": "alpha"}"#;
    let Some((done, calls)) = classify_cfg_run("r = await classify(['apple', 'bean', 'apple'], ['alpha ray', 'beta'])\nr", json!({"legacy": true}), move |_, _, _| Some(reply.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray']", "{done}");
    assert_eq!(calls, ["llm_query_batch_meta"]);
}

#[test]
fn replies_are_read_from_lines_anywhere_with_zero_padded_ids_and_either_case() {
    let reply = "Sure, here are the labels:\n```\n01:a\n2 = B\n 03: A \n4:b.\n```";
    // The trailing dot makes line 4 prose, so the reply is accepted only if every id is covered by real lines.
    let Some((done, _)) = classify_run(CLASSIFY_CODE, move |_, _, _| Some(reply.to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{done}");
    let reply = "Sure, here are the labels:\n```\n01:a\n2 = B\n 03: A \n4:b\n```\nLet me know!";
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, move |_, _, _| Some(reply.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(batches(&calls), 1);
}

#[test]
fn prose_is_ignored_only_when_every_id_is_covered_otherwise_the_whole_chunk_is_asked_again() {
    let call = std::cell::Cell::new(0usize);
    let sizes = std::cell::RefCell::new(Vec::new());
    let Some((done, _)) = run(CLASSIFY_CODE, |name, args| {
        call.set(call.get() + 1);
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        sizes.borrow_mut().push(rows_of(&prompts[0]).len());
        let mode = |c: usize, _: usize, _: &[usize]| (c == 1).then(|| "Here you go.\n1:A\n3:A".to_string());
        Ok(batch_reply(name, labelled(&args[0], &mode, call.get())))
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    // Prose plus two missing ids: nothing from that reply is trusted; the rejected chunk is re-asked at half size.
    assert_eq!(*sizes.borrow(), vec![4, 2], "the second call's first prompt has 2 records");
}

#[test]
fn classify_requeries_only_the_missing_ids_and_keeps_the_valid_ones() {
    let Some((done, sizes, _)) = traced(CLASSIFY_CODE, json!({}), |c, _, _| (c == 1).then(|| "1:A\n3:A".to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![4], vec![2]], "the second ask carries only ids 2 and 4");
}

#[test]
fn classify_accepts_the_old_json_object_reply_as_a_fallback() {
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, |_, _, _| Some(r#"```json
{"1": "ALPHA RAY.", "2": "Beta", "3": "A", "4": " beta "}
```"#.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(calls.len(), 1);
}

#[test]
fn classify_rejects_bad_replies_and_gives_up_loudly() {
    for bad in [
        // an invented id, a repeated id, unknown labels, letters out of range, numbers, free text, prose that misses ids
        "1:A\n2:B\n3:A\n4:B\n9:B",
        "1:A\n1:B\n2:B\n3:A\n4:B",
        "1:gamma\n2:gamma\n3:gamma\n4:gamma",
        "1:Z\n2:Q\n3:Y\n4:X",
        "1:0\n2:1\n3:0\n4:1",
        "1:AB\n2:B\n3:A\n4:B",
        "not json at all",
        "Here you go:\n1:A\n2:B",
        r#"{"1": "alpha ray", "2": "beta", "3": "alpha ray", "4": "beta", "9": "beta"}"#,
        r#"{"1": "gamma", "2": "gamma", "3": "gamma", "4": "gamma"}"#,
    ] {
        let Some((done, calls)) = classify_run(CLASSIFY_CODE, move |_, _, _| Some(bad.to_string())) else { return };
        assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{bad}: {done}");
        assert_eq!(batches(&calls), 3, "first ask plus two retries: {bad}");
    }
}

#[test]
fn classify_does_not_read_not_spam_as_spam() {
    for reply in [r#"{"1": "not spam"}"#, "1:not spam", r#"{"1": "spam, probably"}"#, r#"{"1": "unspam"}"#] {
        let Some((done, _)) = classify_run("await classify(['hello'], ['spam', 'ham'])", move |_, _, _| Some(reply.to_string())) else { return };
        assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{reply}: {done}");
    }
    // The same reply wording in the legacy matcher is the 0.0.3 behaviour it replaces.
    let Some((done, _)) = classify_cfg_run("await classify(['hello'], ['spam', 'ham'])", json!({"legacy": true}), |_, _, _| Some(r#"{"1": "not spam"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['spam']", "{done}");
}

#[test]
fn classify_ambiguous_fragment_is_not_accepted() {
    let code = "await classify(['apple'], ['red ant', 'red bee'])";
    let Some((done, _)) = classify_run(code, |_, _, _| Some(r#"{"1": "red"}"#.to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{done}");
}

#[test]
fn classify_a_partial_fragment_is_a_validation_failure_for_that_id_only() {
    let Some((done, sizes, _)) = traced(CLASSIFY_CODE, json!({}), |c, _, _| (c == 1).then(|| r#"{"1": "alpha ray", "2": "beta", "3": "alpha", "4": "beta"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![4], vec![1]]);
}

#[test]
fn classify_votes_reask_only_disagreeing_items() {
    // Pass 0 and pass 1 are one batch call; they disagree only on item 2. The tie-break asks that item alone.
    let code = "r = await classify(['apple', 'bean', 'corn'], ['alpha ray', 'beta'], votes=3)\nr";
    let Some((done, sizes, _)) = traced(code, json!({}), |call, n, _| (call == 1 && n == 1).then(|| r#"{"1": "alpha ray", "2": "alpha ray", "3": "beta"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![3, 3], vec![1]]);
}

#[test]
fn classify_votes_default_is_one_pass() {
    let Some((_, calls)) = classify_run("await classify(['apple', 'bean', 'corn'], ['alpha ray', 'beta'])", |_, _, _| None) else { return };
    assert_eq!(batches(&calls), 1);
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
    assert_eq!(batches(&calls), 1, "3 chunks, one batch call");
}

#[test]
fn classify_chunk_knobs_follow_the_settings_and_never_split_a_record() {
    let many = "r = await classify([str(i) for i in range(100)], ['alpha ray', 'beta'])\nlen(r)";
    let Some((done, sizes, _)) = traced(many, json!({"chunk_items": 10}), |_, _, _| None) else { return };
    assert_eq!(done["value"], "100", "{done}");
    assert_eq!(sizes, vec![vec![10; 10]]);
    // A character cap of 20: 15-char records go one per chunk, and a 50-char record rides alone, whole.
    let seen = std::cell::RefCell::new(Vec::new());
    let long = "r = await classify(['a' * 15, 'b' * 15, 'c' * 50, 'd' * 5, 'e' * 5], ['alpha ray', 'beta'])\nr";
    let Some((done, _)) = run_cfg(long, json!({"chunk_chars": 20}), |name, args| {
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        seen.borrow_mut().extend(prompts.iter().map(|p| rows_of(p).iter().map(|r| r.1.len()).collect::<Vec<_>>()));
        Ok(batch_reply(name, labelled(&args[0], &|_, _, _| None, 1)))
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta', 'beta', 'beta']", "{done}");
    assert_eq!(*seen.borrow(), vec![vec![50], vec![15], vec![15], vec![5, 5]], "longest prompt sent first");
    // Configured chunks below five are not enlarged by a retry.
    let code = "r = await classify([str(i) for i in range(6)], ['alpha ray', 'beta'])\nr";
    let Some((_, sizes, _)) = traced(code, json!({"chunk_items": 3}), |c, _, _| (c == 1).then(|| "garbage".to_string())) else { return };
    assert_eq!(sizes[0], vec![3, 3]);
    assert_eq!(sizes[1], vec![1; 6], "rejected-whole chunks of 3 are re-asked at 1, never raised to 5");
}

#[test]
fn a_transport_error_re_asks_the_same_chunk_once_unchanged_after_a_backoff_the_engine_waits_for() {
    // 100 items: balanced chunks of 34, 34, 32 (sent longest prompt first). The second sent hits a transport error once.
    let code = "r = await classify([str(i) for i in range(100)], ['alpha ray', 'beta'])\nlen(r)";
    let Some((done, sizes, naps)) = traced(code, json!({"backoff": 1.0}), |c, n, _| (c == 1 && n == 1).then(|| "Error: 429 too many requests".to_string())) else { return };
    assert_eq!(done["value"], "100", "{done}");
    assert_eq!(sizes, vec![vec![34, 32, 34], vec![32]], "only the failed chunk, unchanged, is asked again");
    assert_eq!(naps.len(), 1);
    assert!((1.0..=1.5).contains(&naps[0]), "jittered backoff, waited on the engine's clock (not the cell's compute): {naps:?}");
}

#[test]
fn transport_errors_stop_after_one_re_ask_and_validation_failures_keep_their_own_budget() {
    let Some((done, calls)) = classify_cfg_run("await classify(['apple', 'bean'], ['alpha ray', 'beta'])", json!({}), |_, _, _| Some("Error: 503 unavailable".to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{done}");
    assert_eq!(batches(&calls), 2, "the host already retried the request itself: one re-ask, not nine asks");
    // Per chunk: one transport re-ask, then three answered asks (the transport error did not spend them), then it stops.
    let code = "await classify(['apple', 'bean'], ['alpha ray', 'beta'])";
    let Some((done, calls)) = classify_cfg_run(code, json!({}), |c, _, _| match c {
        1 => Some("Error: timeout".to_string()),
        _ => Some("1:A".to_string()),
    }) else { return };
    assert!(done["error"].as_str().unwrap().contains("without a valid label"), "{done}");
    assert_eq!(batches(&calls), 4);
    let Some((done, calls)) = classify_cfg_run("r = await classify(['apple', 'bean'], ['alpha ray', 'beta'])\nr", json!({}), |c, _, _| match c {
        1 => Some("Error: timeout".to_string()),
        2 => Some("1:A".to_string()),
        _ => None,
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta']", "{done}");
    assert_eq!(batches(&calls), 3);
}

#[test]
fn a_transport_failed_chunk_is_never_regrouped_with_validation_failures() {
    // Two chunks of 2: the first answers partially (id 2 missing), the second hits a 502. The re-asks are two
    // separate prompts: [2] and the unchanged [3, 4].
    let code = "r = await classify(['apple', 'bean', 'corn', 'date'], ['alpha ray', 'beta'])\nr";
    let Some((done, sizes, naps)) = traced(code, json!({"chunk_items": 2}), |c, n, _| match (c, n) {
        (1, 0) => Some("1:A".to_string()),
        (1, 1) => Some("Error: 502 bad gateway".to_string()),
        _ => None,
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta', 'beta']", "{done}");
    // Sent longest prompt first: the failing id alone, then the unchanged transport chunk.
    assert_eq!(sizes, vec![vec![2, 2], vec![1, 2]], "the unchanged transport chunk, then the failing id alone");
    assert_eq!(naps.len(), 1);
}

#[test]
fn a_non_transient_error_is_not_repeated_at_the_same_size() {
    let code = "r = await classify(['apple', 'bean', 'corn', 'date'], ['alpha ray', 'beta'])\nr";
    // 400 on the whole chunk: halved once (2 + 2), no waiting.
    let Some((done, sizes, naps)) = traced(code, json!({}), |c, _, _| (c == 1).then(|| "Error: 400 Bad Request: prompt too long".to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![4], vec![2, 2]]);
    assert!(naps.is_empty());
    // A half that fails again stops the job with the real error.
    let Some((done, sizes, _)) = traced(code, json!({}), |_, _, _| Some("Error: 413 payload too large".to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("refused") && done["error"].as_str().unwrap().contains("413"), "{done}");
    assert_eq!(sizes, vec![vec![4], vec![2, 2], vec![1, 1, 1, 1]], "halved down to single records, then given up");
    // A single record: fail fast, one ask.
    let Some((done, sizes, _)) = traced("await classify(['apple'], ['alpha ray', 'beta'])", json!({}), |_, _, _| Some("Error: 403 forbidden: content filter".to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("refused a request"), "{done}");
    assert_eq!(sizes, vec![vec![1]]);
    // An unsupported-effort error is non-transient too.
    let Some((_, sizes, _)) = traced("await classify(['apple'], ['alpha ray', 'beta'])", json!({}), |_, _, _| Some("Error: Unsupported reasoning effort 'low'".to_string())) else { return };
    assert_eq!(sizes.len(), 1);
}

#[test]
fn identical_records_are_labelled_once_and_restored_in_order() {
    let seen = std::cell::RefCell::new(Vec::new());
    let code = "r = await classify(['apple', 'bean', 'apple', 'bean', 'apple', 'corn'], ['alpha ray', 'beta'])\nr";
    let Some((done, calls)) = run(code, |name, args| {
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        seen.borrow_mut().extend(prompts.iter().flat_map(|p| rows_of(p)).map(|r| r.1));
        Ok(batch_reply(name, labelled(&args[0], &|_, _, _| None, 1)))
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(*seen.borrow(), ["apple", "bean", "corn"]);
    assert_eq!(calls.len(), 1);
}

#[test]
fn records_equal_only_in_their_first_6000_characters_are_not_merged() {
    let code = "long = 'a' * 6000\nr = await classify([long + ' one', long + ' two', long + ' one', 'bean'], ['alpha ray', 'beta'])\nr";
    let Some((done, sizes, _)) = traced(code, json!({}), |_, _, _| None) else { return };
    assert_eq!(done["value"], "['alpha ray', 'alpha ray', 'alpha ray', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![3]], "three distinct records: the two long ones differ past the cut");
}

#[test]
fn a_second_classify_over_the_same_records_costs_nothing_and_every_key_part_matters() {
    let code = "a = await classify(['apple', 'bean'], ['alpha ray', 'beta'])\nb = await classify(['bean', 'apple', 'apple'], ['alpha ray', 'beta'])\nc = await classify(['bean'], ['alpha ray', 'beta'], guidance='other')\n(a, b, c)";
    let Some((done, calls)) = classify_run(code, |_, _, _| None) else { return };
    assert_eq!(done["value"], "(['alpha ray', 'beta'], ['beta', 'alpha ray', 'alpha ray'], ['beta'])", "{done}");
    assert_eq!(batches(&calls), 2, "the repeat is free; new guidance is a new key");
    let Some((_, calls)) = classify_cfg_run(code, json!({"dedupe": false}), |_, _, _| None) else { return };
    assert_eq!(batches(&calls), 3);
    let again = "await classify(['bean'], ['alpha ray', 'beta'])\nawait classify(['bean'], ['alpha ray', 'beta'], votes=2)\nawait classify(['bean'], ['beta', 'alpha ray'])\nawait classify(['bean'], ['alpha ray', 'beta'])";
    let Some((_, calls)) = classify_run(again, |_, _, _| None) else { return };
    assert_eq!(batches(&calls), 3, "votes and the labels tuple are part of the key");
}

#[test]
fn the_cache_key_includes_the_format_the_model_and_the_effective_effort() {
    // The same worker sees the settings change between cells: the cache must not answer across them.
    let dir = std::env::temp_dir().join(format!("worker-cache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new("python3").args(["-I", "-S", "-u", "-c", WORKER]).arg(&dir).arg(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let mut asks = Vec::new();
    for (n, cfg) in [json!({"effort": "low", "model": "m1"}), json!({"effort": "low", "model": "m1"}), json!({"effort": "high", "model": "m1"}), json!({"effort": "low", "model": "m2"}), json!({"effort": "low", "model": "m1", "format": "json"})].into_iter().enumerate() {
        let mut cfg = cfg;
        cfg["dedupe"] = json!(true);
        if cfg.get("format").is_none() {
            cfg["format"] = json!("codes");
        }
        writeln!(stdin, "{}", json!({"op": "run", "code": "await classify(['apple'], ['alpha ray', 'beta'])", "cfg": cfg})).unwrap();
        let mut calls = 0;
        loop {
            line.clear();
            out.read_line(&mut line).unwrap();
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["op"] == "done" {
                assert!(msg["error"].is_null(), "{msg}");
                break;
            }
            calls += 1;
            let args: Vec<String> = msg["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
            writeln!(stdin, "{}", json!({"op": "reply", "value": batch_reply("llm_query_batch_meta", labelled(&args[0], &|_, _, _| None, 1))})).unwrap();
        }
        asks.push((n, calls));
    }
    let _ = child.kill();
    assert_eq!(asks, vec![(0, 1), (1, 0), (2, 1), (3, 1), (4, 1)]);
}

/// Runs `code` with logging on; returns the done frame, the host calls and the log rows.
fn logged_run(code: &str, cfg: Value, mode: impl Fn(usize, usize, &[usize]) -> Option<String>) -> Option<(Value, Vec<String>, Vec<Value>)> {
    let logged = std::cell::RefCell::new(String::new());
    let call = std::cell::Cell::new(0usize);
    let (done, calls) = run_cfg(code, cfg, |name, args| {
        match name {
            "classify_log" => {
                logged.borrow_mut().push_str(&args[0]);
                logged.borrow_mut().push('\n');
                return Ok(String::new());
            }
            "sleep_ms" => return Ok(String::new()),
            _ => {}
        }
        call.set(call.get() + 1);
        Ok(batch_reply(name, labelled(&args[0], &mode, call.get())))
    })?;
    let rows = logged.borrow().lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect();
    Some((done, calls, rows))
}

fn a_cfg(rows: &[Value]) -> &Value {
    rows.iter().find(|r| r["type"] == "call").unwrap()
}

fn of_type<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> {
    rows.iter().filter(|r| r["type"] == kind).collect()
}

#[test]
fn a_configured_effort_the_model_refused_is_logged_as_requested_with_a_fallback_flag() {
    // The user set `none`; the model does not support it, so the engine runs at `medium` and reports both.
    let code = "r = await classify(['apple', 'bean'], ['alpha ray', 'beta'])\nr";
    let cfg = json!({"log": true, "effort": "medium", "requested_effort": "none", "model": "m1"});
    let Some((_, _, rows)) = logged_run(code, cfg, |_, _, _| Some("1:A\n2:B".to_string())) else { return };
    let job = of_type(&rows, "job");
    assert_eq!((job[0]["effort"].as_str(), job[0]["requested_effort"].as_str(), job[0]["effort_fallback"].as_bool()), (Some("medium"), Some("none"), Some(true)));
    for call in of_type(&rows, "call") {
        assert_eq!((call["requested_effort"].as_str(), call["effort_fallback"].as_bool()), (Some("none"), Some(true)), "{call}");
        assert_ne!(call["effort"].as_str(), Some("none"), "{call}");
    }
}

#[test]
fn the_classify_log_has_a_row_per_sub_call_and_per_occurrence_in_the_harness_names_and_no_record_text() {
    let code = "r = await classify(['apple secret', 'bean secret', 'apple secret', 'cherry'], ['alpha ray', 'beta'])\nr";
    let Some((done, calls, rows)) = logged_run(code, json!({"log": true, "effort": "low", "model": "m1"}), |c, _, _| (c == 1).then(|| "1:A".to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    let text = rows.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n");
    for secret in ["apple", "bean", "cherry", "secret"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    assert!(rows.iter().all(|r| ["call", "occ", "job"].contains(&r["type"].as_str().unwrap())), "only the row types the harness reads: {text}");
    let job = of_type(&rows, "job");
    assert_eq!((job.len(), job[0]["status"].as_str(), job[0]["unlabelled"].as_i64(), job[0]["records"].as_i64(), job[0]["to_label"].as_i64()), (1, Some("ok"), Some(0), Some(4), Some(3)));
    assert_eq!((a_cfg(&rows)["chunk_items"].as_i64(), a_cfg(&rows)["chunk_chars"].as_i64(), a_cfg(&rows)["concurrency"].as_i64()), (Some(40), Some(24000), Some(8)));
    assert!(a_cfg(&rows)["prompt_chars"].as_i64().unwrap() > 50 && a_cfg(&rows)["reply_chars"] == 3, "1:A is 3 characters");
    let attempts = of_type(&rows, "call");
    assert_eq!(attempts.len(), 2, "first ask (partial) and the re-ask of the two missing ids: {text}");
    let a = attempts[0];
    assert_eq!((a["chunk_size"].as_i64(), a["records_count"].as_i64(), a["attempt"].as_i64(), a["pass"].as_i64()), (Some(3), Some(1), Some(1), Some(0)));
    assert_eq!((attempts[1]["chunk_size"].as_i64(), attempts[1]["records_count"].as_i64(), attempts[1]["attempt"].as_i64()), (Some(2), Some(2), Some(2)));
    assert!(a["error"].as_str().unwrap().starts_with("reject: missing"), "{a}");
    assert!(attempts[1]["error"].is_null());
    assert_eq!((a["validation_failure"].as_bool(), a["transport_error"].as_bool(), a["refusal"].as_bool()), (Some(true), Some(false), Some(false)));
    assert_eq!((a["input_tokens"].as_i64(), a["output_tokens"].as_i64(), a["cached_tokens"].as_i64(), a["latency_ms"].as_i64()), (Some(120), Some(30), Some(64), Some(250)));
    assert!(a["reasoning_tokens"].is_null());
    assert_eq!((a["effort"].as_str(), a["model"].as_str(), a["format"].as_str(), a["votes"].as_i64(), a["legacy"].as_bool(), a["dedupe"].as_bool()), (Some("low"), Some("m1"), Some("codes"), Some(1), Some(false), Some(true)));
    assert_eq!((a["requested_effort"].as_str(), a["effort_fallback"].as_bool(), job[0]["requested_effort"].as_str(), job[0]["effort_fallback"].as_bool()), (Some("low"), Some(false), Some("low"), Some(false)), "no pin refused: requested is what ran");
    // Per call: its own start (the batch start plus the slot offset the host reports) and its own end.
    let (start, end) = (a["ts_start_ms"].as_i64().unwrap(), a["ts_end_ms"].as_i64().unwrap());
    assert!(start > 1_600_000_000_000 && end - start == 250, "{start} {end}");
    let first = a["results"][0].as_array().unwrap();
    assert_eq!((first[0].as_str().unwrap().len(), first[1].as_str()), (16, Some("alpha ray")));
    // One occurrence row per input item, in order, with the FINAL label after dedupe, and where it was labelled.
    let occs = of_type(&rows, "occ");
    assert_eq!(occs.iter().map(|r| r["occ"].as_i64().unwrap()).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(occs.iter().map(|r| r["label"].as_str().unwrap()).collect::<Vec<_>>(), ["alpha ray", "beta", "alpha ray", "beta"]);
    assert_eq!(occs[0]["h"], occs[2]["h"], "identical records share a hash");
    assert_eq!(occs.iter().map(|r| r["deduped"].as_bool().unwrap()).collect::<Vec<_>>(), [false, false, true, false]);
    assert!(occs.iter().all(|r| r["cached"] == false && r["truncated"] == false));
    assert_eq!((occs[0]["chunk"].clone(), occs[0]["pos"].as_i64()), (a["call_id"].clone(), Some(0)));
    assert_eq!((occs[1]["chunk"].clone(), occs[1]["pos"].as_i64()), (attempts[1]["call_id"].clone(), Some(0)), "labelled on the re-ask");
    assert_eq!(occs[2]["chunk"], occs[0]["chunk"], "a duplicate points at the record that was asked");
    assert_eq!(batches(&calls), 2);
}

#[test]
fn the_log_flags_truncated_and_cached_records_with_or_without_dedupe() {
    let code = "big = 'x' * 7000\nawait classify(['apple', big], ['alpha ray', 'beta'])\nawait classify(['apple'], ['alpha ray', 'beta'])";
    let Some((_, _, rows)) = logged_run(code, json!({"log": true}), |_, _, _| None) else { return };
    let occs = of_type(&rows, "occ");
    assert_eq!(occs.iter().map(|r| r["truncated"].as_bool().unwrap()).collect::<Vec<_>>(), [false, true, false]);
    assert_eq!(occs.iter().map(|r| r["cached"].as_bool().unwrap()).collect::<Vec<_>>(), [false, false, true], "the second call's record came from the cache");
    assert!(occs[2]["chunk"].is_null() && occs[2]["pos"].is_null(), "a cached record was not asked: {}", occs[2]);
    assert_eq!(of_type(&rows, "call").len(), 1, "the cached call made no sub-call");
    let Some((_, _, rows)) = logged_run(code, json!({"log": true, "dedupe": false}), |_, _, _| None) else { return };
    let attempts = of_type(&rows, "call");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["dedupe"].as_bool(), Some(false));
    assert_eq!(of_type(&rows, "occ").len(), 3);
}

#[test]
fn error_fields_are_engine_categories_never_provider_text() {
    let code = "r = await classify(['apple', 'bean', 'corn', 'date'], ['alpha ray', 'beta'])\nr";
    let Some((done, _, rows)) = logged_run(code, json!({"log": true, "chunk_items": 1}), |c, n, _| match (c, n) {
        (1, 0) => Some("Error: 503 Service Unavailable: upstream said SECRET".to_string()),
        (1, 1) => Some("I'm sorry, I can't help with that.".to_string()),
        (1, 2) => Some("Error: 400 Bad Request: context_length_exceeded SECRET".to_string()),
        _ => None,
    }) else { return };
    assert!(done["error"].as_str().unwrap().contains("refused a request"), "{done}");
    let attempts = of_type(&rows, "call");
    let errors: Vec<Option<&str>> = attempts.iter().take(4).map(|r| r["error"].as_str()).collect();
    assert_eq!(errors, [Some("transport: 503"), Some("refusal: the reply declined"), Some("request error: 400 over a size limit"), None]);
    assert_eq!((attempts[0]["transport_error"].as_bool(), attempts[1]["refusal"].as_bool()), (Some(true), Some(true)));
    assert!(!rows.iter().any(|r| r.to_string().contains("SECRET")));
}

#[test]
fn the_legacy_path_logs_the_same_row_types_for_record_level_comparison() {
    let code = "r = await classify(['apple secret', 'bean', 'apple secret'], ['alpha ray', 'beta'])\nr";
    let Some((done, calls, rows)) = logged_run(code, json!({"legacy": true, "log": true}), |_, _, _| None) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray']", "{done}");
    assert_eq!(batches(&calls), 1);
    let text = rows.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n");
    assert!(!text.contains("secret") && !text.contains("apple"), "{text}");
    let attempts = of_type(&rows, "call");
    assert_eq!(attempts.len(), 1);
    assert_eq!((attempts[0]["legacy"].as_bool(), attempts[0]["format"].as_str(), attempts[0]["chunk_size"].as_i64(), attempts[0]["input_tokens"].as_i64()), (Some(true), Some("json"), Some(3), Some(120)));
    let occs = of_type(&rows, "occ");
    assert_eq!(occs.iter().map(|r| r["label"].as_str().unwrap()).collect::<Vec<_>>(), ["alpha ray", "beta", "alpha ray"]);
    assert_eq!(occs[0]["h"], occs[2]["h"]);
    assert_eq!(occs.iter().map(|r| r["pos"].as_i64().unwrap()).collect::<Vec<_>>(), [0, 1, 2], "legacy asks duplicates again: each has its own position");
}

#[test]
fn the_classify_log_hash_is_the_sha256_prefix_of_the_normalised_text() {
    let code = "r = await classify(['  hello   world '], ['alpha ray', 'beta'])\nr";
    let logged = std::cell::RefCell::new(String::new());
    let Some((_, _)) = run_cfg(code, json!({"log": true}), |name, args| {
        if name == "classify_log" {
            logged.borrow_mut().push_str(&args[0]);
            return Ok(String::new());
        }
        Ok(batch_reply(name, labelled(&args[0], &|_, _, _| None, 1)))
    }) else { return };
    let hash = std::process::Command::new("python3").args(["-c", "import hashlib;print(hashlib.sha256(b'hello world').hexdigest()[:16])"]).output().unwrap();
    let want = String::from_utf8(hash.stdout).unwrap().trim().to_string();
    assert!(logged.borrow().contains(&format!("[\"{want}\",\"beta\"]")), "{}", logged.borrow());
}

#[test]
fn without_the_log_setting_nothing_is_logged() {
    let Some((_, calls)) = classify_run(CLASSIFY_CODE, |_, _, _| None) else { return };
    assert!(!calls.iter().any(|c| c == "classify_log"));
}

#[test]
fn a_job_that_cannot_fit_the_cell_is_refused_before_anything_is_sent() {
    // 1100 chunks of one record need 18 batch calls of 64 prompts; the cell has 16 host calls.
    let code = "await classify([str(i) for i in range(1100)], ['alpha ray', 'beta'])";
    let Some((done, calls)) = classify_cfg_run(code, json!({"chunk_items": 1}), |_, _, _| None) else { return };
    let error = done["error"].as_str().unwrap();
    assert!(error.contains("need up to 18 host calls") && error.contains("16 of 16 left"), "{error}");
    assert!(calls.is_empty(), "nothing was sent: {calls:?}");
}

#[test]
fn batches_are_sized_in_utf8_bytes_not_characters() {
    // 125 records of 6000 three-byte characters: 750000 characters (one batch by character count) but about
    // 2.25 MB, over the 2,000,000-byte batch limit the host enforces, so the engine must be given two batches.
    let code = "r = await classify(['€' * 5990 + f'{i:04d}' for i in range(125)], ['alpha ray', 'beta'])\nlen(r)";
    let Some((done, calls)) = classify_cfg_run(code, json!({"chunk_items": 25, "chunk_chars": 150000}), |_, _, _| None) else { return };
    assert_eq!(done["value"], "125", "{done}");
    assert_eq!(batches(&calls), 2);
}

#[test]
fn an_unfinished_job_names_the_unlabelled_items_and_a_rerun_asks_only_for_them() {
    // Two chunks of 2 (four distinct records, the first one twice): the second chunk's provider keeps failing.
    let code = "items = ['apple', 'bean', 'corn', 'date', 'apple']\ntry:\n    await classify(items, ['alpha ray', 'beta'])\nexcept RuntimeError as e:\n    err = str(e)\nerr";
    let Some((done, sizes, _)) = traced(code, json!({"chunk_items": 2}), |_, n, ids| (n == 1 || ids.contains(&3)).then(|| "Error: 503 unavailable".to_string())) else { return };
    let err = done["value"].as_str().unwrap();
    assert!(err.contains("2 of 5 items still without a valid label") && err.contains("[2, 3]") && err.contains("503"), "{err}");
    assert!(err.contains("The 3 labels already given are kept"), "{err}");
    assert_eq!(sizes, vec![vec![2, 2], vec![2]], "the failing chunk once more, unchanged");
    // In the same worker, a second call over the same items asks only for the two that were missing.
    let code = format!("{}\nr = await classify(items, ['alpha ray', 'beta'])\nr", code.replace("\nerr", ""));
    let call = std::cell::Cell::new(0usize);
    let asked = std::cell::RefCell::new(Vec::new());
    let Some((done, _)) = run_cfg(&code, json!({"chunk_items": 2}), |name, args| {
        if name == "sleep_ms" || name == "classify_log" {
            return Ok(String::new());
        }
        call.set(call.get() + 1);
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        let first_job = call.get() <= 2;
        if !first_job {
            asked.borrow_mut().extend(prompts.iter().flat_map(|p| rows_of(p)).map(|r| r.1));
        }
        Ok(batch_reply(name, labelled(&args[0], &|_, _, ids| (first_job && ids.contains(&3)).then(|| "Error: 503 unavailable".to_string()), call.get())))
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta', 'beta', 'alpha ray']", "{done}");
    assert_eq!(*asked.borrow(), ["corn", "date"]);
}

#[test]
fn a_spent_host_budget_mid_job_keeps_what_was_labelled_and_says_to_continue_in_a_new_cell() {
    // Fifteen host calls are spent before classify; the first wave fits in the sixteenth, the re-ask does not.
    let code = "for _ in range(15):\n    await llm_query_batch(['x'])\nawait classify(['apple', 'bean'], ['alpha ray', 'beta'])";
    let call = std::cell::Cell::new(0usize);
    let Some((done, _)) = run_cfg(code, json!({}), |name, args| {
        if name == "llm_query_batch" {
            return Ok("[\"ok\"]".into());
        }
        call.set(call.get() + 1);
        Ok(batch_reply(name, labelled(&args[0], &|c, _, _| (c == 1).then(|| "1:A".to_string()), call.get())))
    }) else { return };
    let err = done["error"].as_str().unwrap();
    assert!(err.contains("1 of 2 items") && err.contains("budget") && err.contains("new cell") && err.contains("[1]") && err.contains("kept"), "{err}");
}

#[test]
fn an_authentication_or_unsupported_setting_error_stops_at_once_without_halving() {
    for error in ["Error: 401 Unauthorized: invalid api key", "Error: 400 Unsupported value: 'low' is not supported with this model", "Error: no active model provider"] {
        let code = "await classify([str(i) for i in range(100)], ['alpha ray', 'beta'])";
        let Some((done, sizes, naps)) = traced(code, json!({}), move |_, _, _| Some(error.to_string())) else { return };
        let err = done["error"].as_str().unwrap();
        assert!(err.contains("asking again cannot help") && err.contains("100 of 100"), "{error}: {err}");
        assert_eq!((sizes.len(), naps.len()), (1, 0), "{error}: one wave, no re-ask, no wait");
    }
}

#[test]
fn a_content_filter_is_isolated_by_halving_and_the_rest_is_labelled() {
    // The provider refuses any prompt holding "corn": halving isolates it; the other records keep their labels.
    let code = "try:\n    await classify(['apple', 'bean', 'corn', 'date'], ['alpha ray', 'beta'])\nexcept RuntimeError as e:\n    err = str(e)\nerr";
    let call = std::cell::Cell::new(0usize);
    let Some((done, _)) = run(code, |name, args| {
        call.set(call.get() + 1);
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        let replies = labelled(&args[0], &|_, _, _| None, call.get());
        let replies = prompts.iter().zip(replies).map(|(p, r)| if p.contains(". corn") { "Error: 400 content_filter: flagged".to_string() } else { r }).collect();
        Ok(batch_reply(name, replies))
    }) else { return };
    let err = done["value"].as_str().unwrap();
    assert!(err.contains("1 of 4 items") && err.contains("[2]") && err.contains("refused a request") && err.contains("3 labels already given"), "{err}");
}

#[test]
fn a_single_letter_label_that_is_not_its_own_code_uses_the_json_format() {
    let seen = std::cell::RefCell::new(Vec::new());
    let host = |code: &str| {
        seen.borrow_mut().clear();
        run(code, |_, args| {
            let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
            seen.borrow_mut().extend(prompts);
            Ok(batch_reply("llm_query_batch_meta", labelled(&args[0], &|_, _, _| Some(r#"{"1": "B"}"#.to_string()), 1)))
        })
    };
    // `A="B"` would invite the wrong letter: JSON, where "B" is the label B.
    let Some((done, _)) = host("await classify(['x'], ['B', 'A'])") else { return };
    assert_eq!(done["value"], "['B']", "{done}");
    assert!(seen.borrow()[0].contains("Allowed labels"), "{}", seen.borrow()[0]);
    // Labels that are their own codes keep the cheap format.
    let Some((_, _)) = host("await classify(['x'], ['a', 'b', 'c'])") else { return };
    assert!(seen.borrow()[0].contains("Codes: A=\"a\" | B=\"b\" | C=\"c\""), "{}", seen.borrow()[0]);
}

#[test]
fn symbol_only_labels_are_allowed_and_matched_exactly() {
    let Some((done, _)) = classify_run("await classify(['good', 'bad'], ['👍', '👎'])", |_, _, _| Some("1:A\n2:B".to_string())) else { return };
    assert_eq!(done["value"], "['👍', '👎']", "{done}");
    let Some((done, _)) = classify_cfg_run("await classify(['good', 'bad'], ['+', '-'])", json!({"format": "json"}), |_, _, _| Some(r#"{"1": "+", "2": "-"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['+', '-']", "{done}");
    // Still distinct and non-empty.
    let Some((done, _)) = classify_run("await classify(['x'], ['👍', ' 👍 '])", |_, _, _| None) else { return };
    assert!(done["error"].as_str().unwrap().contains("distinct"), "{done}");
}

#[test]
fn in_the_json_format_a_number_is_accepted_only_when_it_is_itself_a_label() {
    let Some((done, _)) = classify_cfg_run("await classify(['x', 'y'], ['0', '1'])", json!({"format": "json"}), |_, _, _| Some(r#"{"1": 1, "2": "0"}"#.to_string())) else { return };
    assert_eq!(done["value"], "['1', '0']", "{done}");
}

#[test]
fn numbered_list_style_lines_are_read_but_an_echoed_item_is_not_a_label() {
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, |_, _, _| Some("1. A\n2) B\n3. a\n4:B".to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(batches(&calls), 1);
    // The model echoes the items, then answers: the echo is prose, the answers cover every id.
    let Some((done, calls)) = classify_run(CLASSIFY_CODE, |_, _, _| Some("1. apple\n2. bean\n3. avocado\n4. corn\n1:A\n2:B\n3:A\n4:B".to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'alpha ray', 'beta']", "{done}");
    assert_eq!(batches(&calls), 1);
}

#[test]
fn a_single_label_needs_no_sub_call() {
    let Some((done, calls)) = classify_run("await classify(['a', 'b', 'a'], ['only'])", |_, _, _| None) else { return };
    assert_eq!(done["value"], "['only', 'only', 'only']", "{done}");
    assert!(calls.is_empty());
}

#[test]
fn a_lone_surrogate_in_a_record_does_not_break_the_protocol() {
    let code = "bad = b'caf\\xe9 apple'.decode('utf-8', 'surrogateescape')\nr = await classify([bad, 'bean'], ['alpha ray', 'beta'])\nr";
    let Some((done, _)) = classify_run(code, |_, _, _| None) else { return };
    assert_eq!(done["value"], "['beta', 'beta']", "{done}");
}

#[test]
fn failing_ids_are_regrouped_only_with_ids_at_the_same_ask_count() {
    // Chunks [1,2] and [3,4]. Wave 1: chunk A answers 1 only (id 2 at ask 1), chunk B hits a timeout.
    // Wave 2: B (ask 2) answers 3 only; id 2 is re-asked alone (ask 2). Wave 3: id 4 alone (ask 3) is answered.
    let code = "r = await classify(['apple', 'bean', 'corn', 'date'], ['alpha ray', 'beta'])\nr";
    let Some((done, sizes, _)) = traced(code, json!({"chunk_items": 2}), |c, n, ids| match (c, n) {
        (1, 0) => Some("1:A".to_string()),
        (1, 1) => Some("Error: timeout".to_string()),
        (2, _) if ids == [3, 4] => Some("3:B".to_string()),
        _ => None,
    }) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![2, 2], vec![1, 2], vec![1]]);
}

#[test]
fn no_backoff_wait_when_nothing_is_asked_again() {
    let Some((done, sizes, naps)) = traced("await classify(['apple'], ['alpha ray', 'beta'])", json!({}), |_, _, _| Some("Error: 429".to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("transport error persisted"), "{done}");
    assert_eq!(sizes.len(), 2);
    assert_eq!(naps.len(), 1, "one wait before the re-ask, none before giving up: {naps:?}");
}

#[test]
fn a_chunk_over_the_providers_limit_is_halved_until_it_fits() {
    // The provider takes at most 2 records per prompt: 8 records are asked as 8, then 4 + 4, then 2 + 2 + 2 + 2.
    let code = "r = await classify(['apple', 'bean', 'corn', 'date', 'egg', 'fig', 'gum', 'ham'], ['alpha ray', 'beta'])\nr";
    let Some((done, sizes, _)) = traced(code, json!({}), |_, _, ids| (ids.len() > 2).then(|| "Error: 400 context_length_exceeded".to_string())) else { return };
    assert_eq!(done["value"], "['alpha ray', 'beta', 'beta', 'beta', 'beta', 'beta', 'beta', 'beta']", "{done}");
    assert_eq!(sizes, vec![vec![8], vec![4, 4], vec![2, 2, 2, 2]]);
}

#[test]
fn a_refusal_of_every_request_in_a_wave_stops_instead_of_multiplying_requests() {
    // A provider-wide 400 (a bad parameter, say) on all five chunks: one wave, then a clear error.
    let code = "await classify([str(i) for i in range(200)], ['alpha ray', 'beta'])";
    let Some((done, sizes, _)) = traced(code, json!({}), |_, _, _| Some("Error: 400 Bad Request: invalid parameter".to_string())) else { return };
    let err = done["error"].as_str().unwrap();
    assert!(err.contains("every request of a wave was refused") && err.contains("200 of 200"), "{err}");
    assert_eq!(sizes, vec![vec![40; 5]]);
}

#[test]
fn very_long_labels_keep_the_log_rows_within_the_engines_limit() {
    let code = "labels = ['a' * 900, 'b' * 900]\nr = await classify([str(i) for i in range(200)], labels)\nlen(r)";
    let Some((done, _, rows)) = logged_run(code, json!({"log": true, "chunk_items": 200, "format": "json"}), |_, _, ids| {
        Some(serde_json::to_string(&ids.iter().map(|i| (i.to_string(), json!("a".repeat(900)))).collect::<serde_json::Map<String, Value>>()).unwrap())
    }) else { return };
    assert_eq!(done["value"], "200", "{done}");
    let attempts = of_type(&rows, "call");
    assert!(attempts[0]["results"].is_null() && attempts[0]["records_count"] == 200, "{}", attempts[0]);
    assert!(of_type(&rows, "occ").iter().all(|r| r["label"].as_str().unwrap().len() == 900));
}

#[test]
fn guidance_that_is_not_a_string_is_part_of_the_cache_key_as_text() {
    let code = "a = await classify(['apple'], ['alpha ray', 'beta'], guidance=['short'])\nb = await classify(['apple'], ['alpha ray', 'beta'], guidance=['short'])\n(a, b)";
    let Some((done, calls)) = classify_run(code, |_, _, _| None) else { return };
    assert_eq!(done["value"], "(['alpha ray'], ['alpha ray'])", "{done}");
    assert_eq!(batches(&calls), 1, "the second call is answered from the cache");
}

#[test]
fn a_failed_job_still_logs_every_occurrence_and_a_job_row() {
    let code = "try:\n    await classify(['apple', 'bean', 'corn', 'date', 'apple'], ['alpha ray', 'beta'])\nexcept RuntimeError:\n    pass";
    let Some((_, _, rows)) = logged_run(code, json!({"log": true, "chunk_items": 2}), |_, _, ids| ids.contains(&3).then(|| "Error: 503 unavailable".to_string())) else { return };
    let occs = of_type(&rows, "occ");
    assert_eq!(occs.iter().map(|r| r["label"].as_str()).collect::<Vec<_>>(), [Some("alpha ray"), Some("beta"), None, None, Some("alpha ray")]);
    let job = of_type(&rows, "job");
    assert_eq!((job[0]["status"].as_str(), job[0]["unlabelled"].as_i64(), job[0]["error"].as_str()), (Some("failed"), Some(2), Some("transport: 503")));
    // Refused before sending: a job row says so, the occurrences are there without labels.
    let code = "try:\n    await classify([str(i) for i in range(1100)], ['alpha ray', 'beta'])\nexcept RuntimeError:\n    pass";
    let Some((_, calls, rows)) = logged_run(code, json!({"log": true, "chunk_items": 1}), |_, _, _| None) else { return };
    assert_eq!(batches(&calls), 0);
    assert_eq!((of_type(&rows, "occ").len(), of_type(&rows, "job")[0]["status"].as_str()), (1100, Some("failed")));
    assert!(of_type(&rows, "job")[0]["error"].as_str().unwrap().starts_with("refused before sending"));
}

#[test]
fn preflight_counts_the_worst_case_of_voting() {
    // 400 one-record chunks: passes 0 and 1 need 13 batch calls, the worst-case tie-break passes (votes=4: two)
    // another 13. Refused before anything is sent.
    let code = "await classify([str(i) for i in range(400)], ['alpha ray', 'beta'], votes=4)";
    let Some((done, calls)) = classify_cfg_run(code, json!({"chunk_items": 1}), |_, _, _| None) else { return };
    let err = done["error"].as_str().unwrap();
    assert!(err.contains("need up to 26 host calls") && err.contains("votes=4") && err.contains("fewer votes"), "{err}");
    assert!(calls.is_empty());
}

#[test]
fn a_fatal_error_in_one_batch_stops_the_later_batches_of_the_wave() {
    // 130 one-record chunks: three batch calls of 64, 64, 2. The first answers 401: the others are not sent.
    let code = "await classify([str(i) for i in range(130)], ['alpha ray', 'beta'])";
    let Some((done, calls)) = classify_cfg_run(code, json!({"chunk_items": 1}), |_, _, _| Some("Error: 401 Unauthorized".to_string())) else { return };
    assert!(done["error"].as_str().unwrap().contains("asking again cannot help"), "{done}");
    assert_eq!(batches(&calls), 1);
}

#[test]
fn rejection_notes_and_errors_carry_no_reply_keys_or_provider_text() {
    let seen = std::cell::RefCell::new(Vec::new());
    let call = std::cell::Cell::new(0usize);
    let code = "await classify(['apple', 'bean'], ['alpha ray', 'beta'])";
    let Some((done, _)) = run(code, |name, args| {
        call.set(call.get() + 1);
        seen.borrow_mut().push(args[0].clone());
        Ok(batch_reply(name, labelled(&args[0], &|c, _, _| match c {
            1 => Some(r#"{"1": "alpha ray", "SECRETKEY": "beta"}"#.to_string()),
            2 => Some(r#"{"1": "alpha ray", "1": "beta", "OTHERSECRET": "x"}"#.to_string()),
            _ => Some("Error: 400 Bad Request: {\"error\": {\"param\": \"PROVIDERTEXT\"}}".to_string()),
        }, call.get())))
    }) else { return };
    let err = done["error"].as_str().unwrap();
    let everything = format!("{err}{}", seen.borrow().join(""));
    for secret in ["SECRETKEY", "OTHERSECRET", "PROVIDERTEXT"] {
        assert!(!everything.contains(secret), "{secret}: {everything}");
    }
    assert!(seen.borrow()[1].contains("keys that are not ids"), "{}", seen.borrow()[1]);
}

#[test]
fn five_chunks_refused_for_context_length_recover_by_halving() {
    // Every prompt with more than 20 records is over the provider's context: five chunks of 40 fail together,
    // which is NOT a systemic refusal; halved once, all 200 are labelled.
    let code = "r = await classify([str(i) for i in range(200)], ['alpha ray', 'beta'])\nlen(r)";
    let Some((done, sizes, _)) = traced(code, json!({}), |_, _, ids| (ids.len() > 20).then(|| "Error: 400 context_length_exceeded".to_string())) else { return };
    assert_eq!(done["value"], "200", "{done}");
    assert_eq!(sizes, vec![vec![40; 5], vec![20; 10]]);
}

#[test]
fn a_content_filtered_record_in_an_80_record_chunk_is_isolated_alone() {
    let code = "try:\n    await classify([f'r{i}' for i in range(80)], ['alpha ray', 'beta'])\nexcept RuntimeError as e:\n    err = str(e)\nerr";
    let call = std::cell::Cell::new(0usize);
    let Some((done, _)) = run_cfg(code, json!({"chunk_items": 80}), |name, args| {
        if name == "sleep_ms" || name == "classify_log" {
            return Ok(String::new());
        }
        call.set(call.get() + 1);
        let prompts: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        let replies = labelled(&args[0], &|_, _, _| None, call.get());
        Ok(batch_reply(name, prompts.iter().zip(replies).map(|(p, r)| if p.contains(". r37\n") || p.ends_with(". r37") { "Error: 400 content_filter".to_string() } else { r }).collect()))
    }) else { return };
    let err = done["value"].as_str().unwrap();
    assert!(err.contains("1 of 80 items") && err.contains("[37]") && err.contains("79 labels already given"), "{err}");
}

#[test]
fn a_wave_is_systemic_only_when_every_request_is_refused() {
    // Five chunks: four refused (invalid request), one answered. Not systemic: the four are halved.
    let code = "r = await classify([str(i) for i in range(200)], ['alpha ray', 'beta'])\nlen(r)";
    let Some((done, sizes, _)) = traced(code, json!({}), |c, n, _| (c == 1 && n < 4).then(|| "Error: 400 Bad Request: invalid parameter".to_string())) else { return };
    assert_eq!(done["value"], "200", "{done}");
    assert_eq!(sizes[1].len(), 8, "{sizes:?}");
}

#[test]
fn without_dedupe_each_duplicate_logs_where_it_was_asked() {
    let code = "await classify(['apple', 'bean', 'apple'], ['alpha ray', 'beta'])";
    let Some((_, _, rows)) = logged_run(code, json!({"log": true, "dedupe": false}), |_, _, _| None) else { return };
    let occs = of_type(&rows, "occ");
    assert_eq!(occs.iter().map(|r| r["pos"].as_i64().unwrap()).collect::<Vec<_>>(), [0, 1, 2]);
}

#[test]
fn each_batch_of_a_wave_is_stamped_with_its_own_start() {
    // 70 one-record chunks: two batch calls in one wave; the host takes 300 ms on the first.
    let code = "await classify([str(i) for i in range(70)], ['alpha ray', 'beta'])";
    let call = std::cell::Cell::new(0usize);
    let logged = std::cell::RefCell::new(String::new());
    let Some(_) = run_cfg(code, json!({"log": true, "chunk_items": 1}), |name, args| {
        if name == "classify_log" {
            logged.borrow_mut().push_str(&args[0]);
            logged.borrow_mut().push('\n');
            return Ok(String::new());
        }
        call.set(call.get() + 1);
        if call.get() == 1 {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        Ok(batch_reply(name, labelled(&args[0], &|_, _, _| None, call.get())))
    }) else { return };
    let rows: Vec<Value> = logged.borrow().lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect();
    let starts: Vec<i64> = of_type(&rows, "call").iter().map(|r| r["ts_start_ms"].as_i64().unwrap()).collect();
    assert!(starts[64] - starts[0] >= 290, "the second batch started after the first returned: {} ms", starts[64] - starts[0]);
}

#[test]
fn an_effort_refused_by_the_api_is_logged_once_and_keys_the_cache_by_the_effort_used() {
    // The host reports a call that asked for `low`, was refused (30 ms) and ran at `medium`.
    let dir = std::env::temp_dir().join(format!("worker-effort-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new("python3").args(["-I", "-S", "-u", "-c", WORKER]).arg(&dir).arg(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let mut asks = Vec::new();
    let mut logged = Vec::new();
    // Cell 1 runs with the session's effort `low`; the engine resolves `medium` for cell 2 after the refusal.
    for effort in ["low", "medium"] {
        writeln!(stdin, "{}", json!({"op": "run", "code": "await classify(['apple', 'bean'], ['alpha ray', 'beta'])", "cfg": {"effort": effort, "log": true, "dedupe": true, "format": "codes"}})).unwrap();
        let mut calls = 0;
        loop {
            line.clear();
            out.read_line(&mut line).unwrap();
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["op"] == "done" {
                assert!(msg["error"].is_null(), "{msg}");
                break;
            }
            let args: Vec<String> = msg["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
            if msg["fn"] == "classify_log" {
                logged.extend(args[0].lines().map(|l| serde_json::from_str::<Value>(l).unwrap()));
                writeln!(stdin, "{}", json!({"op": "reply", "value": ""})).unwrap();
                continue;
            }
            calls += 1;
            let rows: Vec<Value> = labelled(&args[0], &|_, _, _| None, 1).into_iter().map(|t| json!({"t": t, "e": null, "i": 10, "o": 5, "c": 0, "r": null, "ms": 200, "s": 0, "f": "medium", "q": "low", "x": 30})).collect();
            writeln!(stdin, "{}", json!({"op": "reply", "value": json!(rows).to_string()})).unwrap();
        }
        asks.push(calls);
    }
    let _ = child.kill();
    assert_eq!(asks, [1, 0], "the second cell, at the effort really used, is answered from the cache");
    let calls = of_type(&logged, "call");
    assert_eq!(calls.len(), 2, "the refused request and the real one");
    assert_eq!((calls[0]["error"].as_str(), calls[0]["latency_ms"].as_i64(), calls[0]["effort"].as_str()), (Some("request failed: effort refused"), Some(30), Some("low")));
    assert_eq!((calls[1]["effort"].as_str(), calls[1]["requested_effort"].as_str(), calls[1]["error"].is_null()), (Some("medium"), Some("low"), true));
    assert_eq!((calls[0]["effort_fallback"].as_bool(), calls[1]["effort_fallback"].as_bool()), (Some(true), Some(true)), "both rows say the model refused the configured effort");
    let occs = of_type(&logged, "occ");
    assert_eq!((occs[0]["effort"].as_str(), occs[2]["cached"].as_bool(), occs[2]["effort"].as_str()), (Some("medium"), Some(true), Some("medium")));
}

#[test]
fn chunks_are_balanced_without_a_one_record_tail() {
    let code = "await classify([str(i) for i in range(41)], ['alpha ray', 'beta'])";
    let Some((_, sizes, _)) = traced(code, json!({}), |_, _, _| None) else { return };
    let mut first = sizes[0].clone();
    first.sort();
    assert_eq!((sizes.len(), first), (1, vec![20, 21]));
}

// ---- 0.0.4 defaults: json format, dedupe off, data-shaped chunk size (settings sent untouched) ----

/// Runs `code` with the shipped defaults plus `cfg` (log on); returns the done frame, the item count of every
/// prompt and the log rows.
fn defaults_run(code: &str, cfg: Value) -> Option<(Value, Vec<usize>, Vec<Value>, Vec<String>)> {
    let mut cfg = cfg;
    cfg["log"] = json!(true);
    let (sizes, rows, prompts) = (std::cell::RefCell::new(Vec::new()), std::cell::RefCell::new(Vec::new()), std::cell::RefCell::new(Vec::new()));
    let call = std::cell::Cell::new(0usize);
    let (done, _) = run_raw(code, cfg, |name, args| {
        if name == "classify_log" {
            rows.borrow_mut().extend(args[0].lines().map(|l| serde_json::from_str::<Value>(l).unwrap()));
            return Ok(String::new());
        }
        call.set(call.get() + 1);
        let ps: Vec<String> = serde_json::from_str(&args[0]).unwrap();
        sizes.borrow_mut().extend(ps.iter().map(|p| rows_of(p).len()));
        prompts.borrow_mut().extend(ps);
        Ok(batch_reply(name, labelled(&args[0], &|_, _, _| None, call.get())))
    })?;
    Some((done, sizes.into_inner(), rows.into_inner(), prompts.into_inner()))
}

/// 200 distinct records of exactly `len` characters, all labelled `alpha ray`, against `labels` distinct labels.
fn shaped_code(len: usize, labels: usize) -> String {
    let extra: Vec<String> = (0..labels.saturating_sub(2)).map(|k| format!("'l{k}'")).collect();
    format!("labels = ['alpha ray', 'beta', {}]\nr = await classify([('a%d' % i).ljust({len}, 'x') for i in range(200)], labels[:{labels}])\nlen(r)", extra.join(", "))
}

fn chunk_choice(len: usize, labels: usize, cfg: Value) -> (Vec<usize>, Value) {
    let (done, sizes, rows, _) = defaults_run(&shaped_code(len, labels), cfg).expect("python");
    assert!(done["error"].is_null(), "{done}");
    let job = rows.into_iter().find(|r| r["type"] == "job").unwrap();
    (sizes, job)
}

#[test]
fn defaults_are_json_without_dedupe() {
    let Some((done, sizes, rows, prompts)) = defaults_run("await classify(['apple', 'apple', 'bean'], ['alpha ray', 'beta'])", json!({})) else { return };
    assert!(done["error"].is_null(), "{done}");
    assert_eq!(sizes, vec![3], "identical records are each asked");
    assert!(prompts.iter().all(|p| !p.contains("\nCodes: ") && p.contains("Allowed labels")), "json format");
    let job = rows.iter().find(|r| r["type"] == "job").unwrap();
    assert_eq!((job["format"].as_str(), job["dedupe"].as_bool(), job["to_label"].as_i64(), job["records"].as_i64()), (Some("json"), Some(false), Some(3), Some(3)));
    // Per-occurrence logging works without dedupe: one row per input, none marked as deduplicated.
    let occs: Vec<&Value> = rows.iter().filter(|r| r["type"] == "occ").collect();
    assert_eq!((occs.len(), occs.iter().any(|r| r["deduped"] == true)), (3, false));
    // Opt-in: dedupe and the compact codes format.
    let Some((_, sizes, rows, prompts)) = defaults_run("await classify(['apple', 'apple', 'bean'], ['alpha ray', 'beta'])", json!({"dedupe": true, "format": "codes"})) else { return };
    assert_eq!(sizes, vec![2]);
    assert!(prompts.iter().all(|p| p.contains("\nCodes: ")));
    assert!(rows.iter().any(|r| r["type"] == "occ" && r["deduped"] == true));
}

#[test]
fn chunk_size_follows_the_data_when_the_user_has_not_set_it() {
    // Few labels, short records: 80 items / 48000 chars, balanced (200 -> 67+67+66).
    let (sizes, job) = chunk_choice(100, 2, json!({}));
    assert_eq!(sizes, vec![67, 67, 66]);
    assert_eq!((job["chunk_items"].as_i64(), job["chunk_chars"].as_i64(), job["chunk_source"].as_str()), (Some(80), Some(48000), Some("default-80")));
    // Label boundary: 6 labels is still wide, 7 is not.
    let (sizes, job) = chunk_choice(100, 6, json!({}));
    assert_eq!((sizes.len(), job["chunk_source"].as_str()), (3, Some("default-80")));
    let (sizes, job) = chunk_choice(100, 7, json!({}));
    assert_eq!((sizes, job["chunk_items"].as_i64(), job["chunk_chars"].as_i64(), job["chunk_source"].as_str()), (vec![40; 5], Some(40), Some(24000), Some("default-40")));
    // Median length boundary: 600 characters is short, 601 is not.
    let (_, job) = chunk_choice(600, 2, json!({}));
    assert_eq!((job["chunk_items"].as_i64(), job["chunk_source"].as_str()), (Some(80), Some("default-80")));
    let (_, job) = chunk_choice(601, 2, json!({}));
    assert_eq!((job["chunk_items"].as_i64(), job["chunk_chars"].as_i64(), job["chunk_source"].as_str()), (Some(40), Some(24000), Some("default-40")));
}

#[test]
fn an_explicit_chunk_setting_wins_and_is_logged_as_user() {
    let (sizes, job) = chunk_choice(100, 2, json!({"chunk_items": 10}));
    assert_eq!(sizes, vec![10; 20]);
    assert_eq!((job["chunk_items"].as_i64(), job["chunk_source"].as_str()), (Some(10), Some("user")));
    // Long records and many labels would take 40; an explicit 80 is honoured as is.
    let (sizes, job) = chunk_choice(100, 9, json!({"chunk_items": 80, "chunk_chars": 48000}));
    assert_eq!((sizes.len(), job["chunk_items"].as_i64(), job["chunk_chars"].as_i64(), job["chunk_source"].as_str()), (3, Some(80), Some(48000), Some("user")));
    let (_, job) = chunk_choice(100, 2, json!({"chunk_chars": 3000}));
    assert_eq!((job["chunk_items"].as_i64(), job["chunk_chars"].as_i64(), job["chunk_source"].as_str()), (Some(80), Some(3000), Some("user")));
}

#[test]
fn legacy_keeps_the_0_0_3_chunking() {
    let (_, job) = chunk_choice(100, 2, json!({"legacy": true}));
    assert_eq!((job["chunk_items"].as_i64(), job["chunk_chars"].as_i64(), job["chunk_source"].as_str(), job["legacy"].as_bool()), (Some(40), Some(24000), Some("default-40"), Some(true)));
}

#[test]
fn the_wide_chunk_prompt_fits_the_batch_and_prompt_limits() {
    // 80 records of 600 characters: one 48000-character prompt per chunk, far under the 200000-character limit,
    // and 64 such prompts stay under what a batch call carries once split by bytes (the worker groups them).
    let (done, sizes, rows, prompts) = defaults_run(&shaped_code(600, 2), json!({})).expect("python");
    assert!(done["error"].is_null(), "{done}");
    assert_eq!(sizes.len(), 3);
    assert!(prompts.iter().all(|p| p.len() < 60_000), "{:?}", prompts.iter().map(String::len).collect::<Vec<_>>());
    assert!(rows.iter().filter(|r| r["type"] == "call").all(|r| r["prompt_chars"].as_i64().unwrap() < 200_000));
}
