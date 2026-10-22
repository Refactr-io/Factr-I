//! Optional OpenTelemetry export of finished turns (`monitoring.export.otlp`, off until enabled).
//!
//! When an endpoint is configured, every turn the observer finishes is sent to `<endpoint>/v1/traces` as
//! OTLP/HTTP JSON: the turn as the root span and its model calls, tool calls and engine steps as children,
//! with the GenAI semantic-convention attributes for model, provider and token counts. Only ids, counts,
//! durations, statuses and fixed labels leave the machine: never prompts, replies, tool arguments or error
//! text. Export is best effort on its own thread (a bounded queue; a full queue or a failed POST drops the
//! turn), never on the path that writes `factr.db`, and the thread starts only when the first turn finishes
//! with the destination enabled, so the setting costs nothing while it is off. The destination is the
//! backend's existing `monitoring.export.otlp` (enabled, endpoint, `headers_env`: header name to the name of
//! the environment variable or `.env` entry that holds its value).

use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

const QUEUE: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SPANS: usize = 2000;

static EXPORTER: OnceLock<SyncSender<String>> = OnceLock::new();

/// A turn finished: queue it for export when an endpoint is configured.
pub(super) fn turn_finished(home: &Path, run_id: &str) {
    let export = &factr_base::factr_config::current().observability_export;
    if !export.enabled || export.endpoint.is_none() {
        return;
    }
    let tx = EXPORTER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<String>(QUEUE);
        let db = home.join("factr.db");
        let started = std::thread::Builder::new().name("factr-otlp".into()).spawn(move || {
            let client = reqwest::blocking::Client::builder().timeout(TIMEOUT).build().ok();
            while let Ok(run) = rx.recv() {
                let cfg = factr_base::factr_config::current().observability_export.clone();
                let (true, Some(endpoint), Some(client)) = (cfg.enabled, cfg.endpoint, client.as_ref()) else { continue };
                let Some(body) = payload_from(&db, &run) else { continue };
                let mut request = client.post(format!("{endpoint}/v1/traces")).json(&body);
                // `headers_env` names the variable holding each value: credentials never sit in config.yaml.
                for (header, variable) in &cfg.headers_env {
                    if let Some(value) = std::env::var(variable).ok().or_else(|| crate::factr_env::resolve(variable)).filter(|v| !v.is_empty()) {
                        request = request.header(header.as_str(), value);
                    }
                }
                if let Err(err) = request.send().and_then(|r| r.error_for_status()) {
                    eprintln!("factr-otlp: export failed: {}", err.without_url());
                }
            }
        });
        if started.is_err() {
            eprintln!("factr-otlp: could not start the export thread");
        }
        tx
    });
    let _ = tx.try_send(run_id.to_string());
}

fn payload_from(db: &PathBuf, run_id: &str) -> Option<Value> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    payload(&conn, run_id)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A stable 16-byte trace id / 8-byte span id derived from an engine id.
fn id_hex(id: &str, bytes: usize) -> String {
    hex(&Sha256::digest(id.as_bytes())[..bytes])
}

fn attr(key: &str, value: Value) -> Value {
    let typed = match value {
        Value::String(s) => json!({ "stringValue": s }),
        Value::Bool(b) => json!({ "boolValue": b }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "intValue": n.to_string() }),
        Value::Number(n) => json!({ "doubleValue": n }),
        other => json!({ "stringValue": other.to_string() }),
    };
    json!({ "key": key, "value": typed })
}

fn nanos(ms: i64) -> String {
    (ms.max(0) as i128 * 1_000_000).to_string()
}

fn status(status: &str) -> Value {
    // 1 = ok, 2 = error; the engine's error text is never sent.
    json!({ "code": if status == "error" { 2 } else { 1 } })
}

/// Scalar attributes of an engine span (`attributes` holds ids, counts and fixed labels), as `factr.<key>`.
fn engine_attributes(raw: &str) -> Vec<Value> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) else { return Vec::new() };
    map.into_iter()
        .filter(|(_, v)| v.is_boolean() || v.is_number() || v.as_str().is_some_and(|s| s.len() <= 80))
        .take(20)
        .map(|(k, v)| attr(&format!("factr.{k}"), v))
        .collect()
}

/// The OTLP/HTTP JSON body for one finished turn, or `None` when the turn is unknown.
pub(super) fn payload(conn: &Connection, run_id: &str) -> Option<Value> {
    let turn = conn
        .query_row(
            "SELECT id, session_id, parent_id, root_id, kind, model, provider, status, started_at_ms, COALESCE(ended_at_ms, started_at_ms), input_tokens, output_tokens, cache_read_tokens, cost_usd FROM fact_turn WHERE id=?1",
            [run_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?, r.get::<_, i64>(8)?, r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?, r.get::<_, i64>(11)?, r.get::<_, i64>(12)?, r.get::<_, Option<f64>>(13)?,
                ))
            },
        )
        .ok()?;
    let (id, session, parent, root, kind, model, provider, st, start, end, input, output, cache, cost) = turn;
    let trace = id_hex(&root, 16);
    let mut spans = vec![{
        let mut attrs = vec![
            attr("gen_ai.operation.name", json!(kind)),
            attr("gen_ai.system", json!(provider)),
            attr("gen_ai.request.model", json!(model)),
            attr("gen_ai.usage.input_tokens", json!(input)),
            attr("gen_ai.usage.output_tokens", json!(output)),
            attr("factr.cache_read_tokens", json!(cache)),
            attr("session.id", json!(id_hex(&session, 8))),
        ];
        if let Some(c) = cost {
            attrs.push(attr("factr.cost_usd", json!(c)));
        }
        json!({
            "traceId": trace, "spanId": id_hex(&id, 8), "parentSpanId": parent.map(|p| id_hex(&p, 8)).unwrap_or_default(),
            "name": kind, "kind": 1, "startTimeUnixNano": nanos(start), "endTimeUnixNano": nanos(end),
            "attributes": attrs, "status": status(&st),
        })
    }];
    let mut stmt = conn
        .prepare("SELECT id, parent_id, kind, name, status, started_at_ms, COALESCE(ended_at_ms, started_at_ms), input_tokens, output_tokens, COALESCE(model,''), COALESCE(provider,''), attributes FROM spans WHERE run_id=?1 OR root_id=?1 ORDER BY started_at_ms LIMIT ?2")
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![run_id, MAX_SPANS as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, i64>(7)?, r.get::<_, i64>(8)?, r.get::<_, String>(9)?, r.get::<_, String>(10)?, r.get::<_, String>(11)?))
        })
        .ok()?;
    for row in rows.flatten() {
        let (sid, sparent, skind, name, sstatus, sstart, send, sin, sout, smodel, sprovider, sattrs) = row;
        let mut attrs = vec![attr("factr.span.kind", json!(skind))];
        if sin > 0 || sout > 0 {
            attrs.push(attr("gen_ai.usage.input_tokens", json!(sin)));
            attrs.push(attr("gen_ai.usage.output_tokens", json!(sout)));
        }
        if !smodel.is_empty() {
            attrs.push(attr("gen_ai.request.model", json!(smodel)));
        }
        if !sprovider.is_empty() {
            attrs.push(attr("gen_ai.system", json!(sprovider)));
        }
        attrs.extend(engine_attributes(&sattrs));
        spans.push(json!({
            "traceId": trace, "spanId": id_hex(&sid, 8), "parentSpanId": id_hex(&sparent, 8),
            "name": name, "kind": 1, "startTimeUnixNano": nanos(sstart), "endTimeUnixNano": nanos(send),
            "attributes": attrs, "status": status(&sstatus),
        }));
    }
    Some(json!({
        "resourceSpans": [{
            "resource": { "attributes": [attr("service.name", json!("factr-i")), attr("service.version", json!(env!("CARGO_PKG_VERSION")))] },
            "scopeSpans": [{ "scope": { "name": "factr.observability" }, "spans": spans }],
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        super::super::schema::open(&mut conn).unwrap();
        conn
    }

    #[test]
    fn a_turn_becomes_a_trace_with_usage_and_no_text() {
        let conn = db();
        conn.execute("INSERT INTO fact_turn(id,session_id,parent_id,root_id,kind,title,model,provider,status,started_at_ms,ended_at_ms,input_tokens,output_tokens) VALUES('run1','sess1',NULL,'run1','invoke_agent','SECRET TITLE','gpt-x','openai','complete',1000,3000,120,30)", []).unwrap();
        conn.execute("INSERT INTO spans(id,run_id,parent_id,root_id,kind,name,status,started_at_ms,ended_at_ms,input_tokens,output_tokens,error,model,provider,attributes) VALUES('sp1','run1','run1','run1','execute_tool','bash','error',1500,2500,0,0,'SECRET ERROR TEXT','','','{\"exit\":2,\"note\":\"ok\"}')", []).unwrap();
        let body = payload(&conn, "run1").unwrap();
        let spans = body["resourceSpans"][0]["scopeSpans"][0]["spans"].as_array().unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0]["name"], "invoke_agent");
        assert_eq!(spans[0]["parentSpanId"], "");
        assert_eq!(spans[0]["startTimeUnixNano"], "1000000000");
        assert_eq!(spans[1]["parentSpanId"], spans[0]["spanId"], "children hang off the turn");
        assert_eq!(spans[1]["traceId"], spans[0]["traceId"]);
        assert_eq!(spans[1]["status"]["code"], 2);
        let text = body.to_string();
        assert!(!text.contains("SECRET"), "titles and error text never leave the machine");
        assert!(text.contains("gen_ai.usage.input_tokens") && text.contains("\"gpt-x\"") && text.contains("factr.exit"));
        assert_eq!(id_hex("run1", 16).len(), 32);
        assert_eq!(id_hex("run1", 8).len(), 16);
    }

    #[test]
    fn an_unknown_turn_has_nothing_to_export() {
        assert!(payload(&db(), "missing").is_none());
    }
}
