//! CONTRACT SWEEP: calls every method in the vendored Factr contract over the real JSON-RPC
//! WebSocket (in-process gateway, stand-in daemon socket, no Python backend, no model) with the
//! smallest params its schema allows, classifies each reply and compares with a committed golden.
//!
//! Classes: native_ok | native_params_error (-32602) | native_error (other engine error)
//!   | forwarded_unavailable ("not supported by engine") | not_found (BUG) | timeout (BUG) | crash (BUG).
//!
//! Fails when a method changes class, a contract method is new/removed, any method is
//! not_found/timeout/crash, or a forwarded_unavailable method is missing from
//! tests/forwarded_allowlist.txt (`method: reason`, one line each).
//!
//! REGENERATE after an intentional change:
//!   UPDATE_GOLDEN=1 nice -n 10 cargo test -p factr-gateway --test contract_sweep -j 4
//! (rewrites contract_sweep.golden.json; appends new forwarded methods to the allowlist with a
//! family reason you should then review; BUG classes still fail even when regenerating.)

// A stand-in engine daemon on a Unix socket: these tests are Unix-only.
#![cfg(unix)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use factr_gateway::{Config, Gateway};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn resolve(root: &Value, v: &Value, depth: usize) -> Value {
    if depth > 8 {
        return json!({});
    }
    if let Some(r) = v["$ref"].as_str() {
        let mut cur = root;
        for part in r.trim_start_matches("#/").split('/') {
            cur = &cur[part];
        }
        return resolve(root, cur, depth + 1);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(opts) = v[key].as_array() {
            let pick = opts.iter().find(|o| o["type"] != "null").or(opts.first());
            return resolve(root, pick.unwrap_or(&json!({})), depth + 1);
        }
    }
    if let Some(all) = v["allOf"].as_array() {
        return resolve(root, all.first().unwrap_or(&json!({})), depth + 1);
    }
    v.clone()
}

/// Smallest value satisfying `schema`; `sid` fills any `session_id` string.
fn minimal(root: &Value, schema: &Value, name: &str, sid: &str, depth: usize) -> Value {
    let s = resolve(root, schema, depth);
    if let Some(c) = s.get("const") {
        return c.clone();
    }
    if let Some(e) = s["enum"].as_array().and_then(|e| e.first()) {
        return e.clone();
    }
    let ty = match &s["type"] {
        Value::Array(a) => a.iter().find(|t| *t != "null").cloned().unwrap_or(json!("null")),
        t => t.clone(),
    };
    match ty.as_str() {
        Some("string") => json!(if name == "session_id" { sid } else { "x" }),
        Some("integer") | Some("number") => json!(s["minimum"].as_i64().unwrap_or(0).max(0)),
        Some("boolean") => json!(false),
        Some("array") => json!([]),
        Some("null") => Value::Null,
        _ => {
            let mut o = serde_json::Map::new();
            if let Some(req) = s["required"].as_array() {
                for k in req.iter().filter_map(Value::as_str) {
                    o.insert(k.into(), minimal(root, &s["properties"][k], k, sid, depth + 1));
                }
            }
            Value::Object(o)
        }
    }
}

struct Sock(tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>);

async fn connect(port: u16, token: &str) -> Sock {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/api/ws?token={token}")).await.expect("ws connect");
    Sock(ws)
}

impl Sock {
    /// `Ok(reply)`, `Err("timeout")` or `Err("crash")`.
    async fn call(&mut self, id: u64, method: &str, params: Value, wait: Duration) -> Result<Value, &'static str> {
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        if self.0.send(Message::Text(req.to_string())).await.is_err() {
            return Err("crash");
        }
        let fut = async {
            loop {
                match self.0.next().await {
                    Some(Ok(Message::Text(t))) => {
                        let f: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                        if f["id"] == id {
                            return Ok(f);
                        }
                    }
                    Some(Ok(_)) => {}
                    _ => return Err("crash"),
                }
            }
        };
        tokio::time::timeout(wait, fut).await.unwrap_or(Err("timeout"))
    }
}

fn classify(r: &Result<Value, &'static str>) -> &'static str {
    match r {
        Err("timeout") => "timeout",
        Err(_) => "crash",
        Ok(f) if f.get("result").is_some() => "native_ok",
        Ok(f) => match f["error"]["code"].as_i64() {
            Some(-32601) if f["error"]["data"]["reason"] == "not_supported_by_engine" => "forwarded_unavailable",
            Some(-32601) => "not_found",
            Some(-32602) => "native_params_error",
            _ => "native_error",
        },
    }
}

fn family_reason(method: &str) -> String {
    let f = method.split('.').next().unwrap_or("");
    match f {
        "cron" => "cron scheduler lives in the Python backend".into(),
        "skills" | "hub" => "skills are served by the Python backend".into(),
        "voice" | "tts" | "stt" => "voice pipeline is served by the Python backend".into(),
        "pets" | "pet" => "pets are served by the Python backend".into(),
        "profiles" | "profile" => "multi-profile management is served by the Python backend".into(),
        _ => format!("{f}.* has no native engine implementation; the Python backend serves it"),
    }
}

pub fn contract_methods() -> (Value, Vec<(String, Value)>) {
    let c: Value = serde_json::from_str(include_str!("../contract/gateway-contract.openrpc.json")).unwrap();
    let ms = c["methods"].as_array().unwrap().iter().map(|m| (m["name"].as_str().unwrap().to_string(), m.clone())).collect();
    (c, ms)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_contract_method_is_served_or_explicitly_forwarded() {
    let home = std::env::temp_dir().join(format!("contract-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("factr")).unwrap();
    // Nothing the sweep does may touch the real home.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("FACTR_CONFIG_HOME", home.join("factr"));
    }
    // Stand-in daemon: acknowledges every request so session-scoped methods get an answer.
    let socket = home.join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
                    let reply = match req["type"].as_str() {
                        Some("state") => json!({"type":"state","id":req["id"],"session_id":"session_sweep","is_processing":false}),
                        Some("get_history") => json!({"type":"history","id":req["id"],"session_id":"session_sweep","messages":[],"provider_name":"p","provider_model":"m"}),
                        Some("compact") => json!({"type":"compact_result","id":req["id"],"success":true,"message":"ok"}),
                        Some("split") => json!({"type":"split_response","id":req["id"],"new_session_id":"session_sweep"}),
                        Some("cancel" | "soft_interrupt" | "rename_session") => json!({"type":"ack","id":req["id"]}),
                        // A new chat is always pinned to a model (session.create sends set_model).
                        Some("set_model") => json!({"type":"model_changed","id":req["id"],"model":req["model"],"provider_name":"p"}),
                        _ => json!({"type":"done","id":req["id"]}),
                    };
                    let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                }
            });
        }
    });
    let token = "contract-sweep-token-0123456789-abcdefghij".to_string();
    let gateway = Gateway::bind(Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: token.clone(),
        version: "test".into(),
        legacy_socket: socket,
        default_cwd: home.to_string_lossy().into(),
        allow_non_loopback: false,
        provider: "p".into(),
        model: "m".into(),
        reasoning_efforts: Vec::new(),
        profile_model_applies: true,
        home: home.to_string_lossy().into(),
        complete: None,
        features: None,
        learning: None,
    })
    .await
    .expect("gateway binds");
    let port = gateway.local_addr().port();
    tokio::spawn(gateway.serve());

    let (contract, methods) = contract_methods();
    let mut sock = connect(port, &token).await;
    let mut id = 1u64;
    let created = sock.call(id, "session.create", json!({}), Duration::from_secs(3)).await;
    let sid = created.ok().and_then(|f| f["result"]["session_id"].as_str().map(String::from)).unwrap_or_else(|| "x".into());

    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for (name, m) in &methods {
        let schema = &m["params"][0]["schema"];
        let params = if schema.is_null() { json!({}) } else { minimal(&contract, schema, "", &sid, 0) };
        id += 1;
        let r = sock.call(id, name, params, Duration::from_secs(3)).await;
        let class = classify(&r);
        if class == "crash" {
            sock = connect(port, &token).await;
        }
        table.insert(name.clone(), class.into());
    }

    let golden_path = dir().join("contract_sweep.golden.json");
    let allow_path = dir().join("forwarded_allowlist.txt");
    let bugs: Vec<_> = table.iter().filter(|(_, c)| matches!(c.as_str(), "not_found" | "timeout" | "crash")).collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for c in table.values() {
        *counts.entry(c.as_str()).or_default() += 1;
    }
    eprintln!("contract sweep: {counts:?}");
    assert!(bugs.is_empty(), "BUG classes (engine must answer or explicitly forward): {bugs:?}");

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden_path, serde_json::to_string_pretty(&table).unwrap() + "\n").unwrap();
        let mut lines: Vec<String> = std::fs::read_to_string(&allow_path).unwrap_or_default().lines().map(String::from).collect();
        let known: Vec<String> = lines.iter().filter_map(|l| l.split(':').next().map(|s| s.trim().to_string())).collect();
        let mut fwd: Vec<&String> = table.iter().filter(|(_, c)| *c == "forwarded_unavailable").map(|(m, _)| m).collect();
        fwd.sort();
        for m in fwd.into_iter().filter(|m| !known.contains(m)) {
            lines.push(format!("{m}: {}", family_reason(m)));
        }
        lines.retain(|l| l.split(':').next().is_some_and(|m| table.get(m.trim()).is_some_and(|c| c == "forwarded_unavailable")));
        lines.sort();
        std::fs::write(&allow_path, lines.join("\n") + "\n").unwrap();
    }

    let golden: BTreeMap<String, String> = serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("golden exists; run with UPDATE_GOLDEN=1")).unwrap();
    let mut diffs = Vec::new();
    for (m, c) in &table {
        match golden.get(m) {
            None => diffs.push(format!("{m}: NEW contract method, now {c}")),
            Some(g) if g != c => diffs.push(format!("{m}: {g} -> {c}")),
            _ => {}
        }
    }
    for m in golden.keys().filter(|m| !table.contains_key(*m)) {
        diffs.push(format!("{m}: removed from contract"));
    }
    assert!(diffs.is_empty(), "wiring changed (fix, or UPDATE_GOLDEN=1 if intended):\n{}", diffs.join("\n"));

    let allow = std::fs::read_to_string(&allow_path).expect("forwarded_allowlist.txt");
    let allowed: Vec<&str> = allow.lines().filter(|l| l.contains(": ") ).filter_map(|l| l.split(':').next()).collect();
    let missing: Vec<_> = golden.iter().filter(|(m, c)| *c == "forwarded_unavailable" && !allowed.contains(&m.as_str())).map(|(m, _)| m).collect();
    assert!(missing.is_empty(), "forwarded methods without an allowlist reason: {missing:?}");
    std::fs::remove_dir_all(home).ok();
}
