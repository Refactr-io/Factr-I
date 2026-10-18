//! `message_agent` through the engine's existing SDK tool callback, using the upstream relay envelope.
//! The tool acknowledges a durable queue write. The Desktop returns the later answer to the sender's session.
use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    let home = factr_base::factr_config::home().unwrap_or_default();
    if home
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n == "profiles")
    {
        home.parent()
            .and_then(Path::parent)
            .unwrap_or(&home)
            .to_path_buf()
    } else {
        home
    }
}
fn own() -> String {
    std::env::var("FACTR_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".into())
}
fn handle(name: &str) -> &str {
    if name == "default" { "factr" } else { name }
}
fn yaml(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or(Value::Null)
}
fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}
fn aliases(s: &str) -> Vec<String> {
    let mut slug = String::new();
    let mut collapsed = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
            slug.push(c);
            collapsed.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    [slug.trim_matches('-').to_string(), collapsed]
        .into_iter()
        .filter(|s| {
            valid(s) && !["all", "everyone", "user", "default", "factr"].contains(&s.as_str())
        })
        .collect()
}
fn roster(base: &Path) -> Vec<Value> {
    let mut rows = vec![];
    let mut dirs = vec![("default".to_string(), base.to_path_buf())];
    if let Ok(entries) = std::fs::read_dir(base.join("profiles")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if valid(&name)
                && name != "default"
                && path.is_dir()
                && !base.join("profiles/.deleted").join(&name).exists()
                && ["profile.yaml", "config.yaml", "SOUL.md", "state.db"]
                    .iter()
                    .any(|f| path.join(f).is_file())
            {
                dirs.push((name, path));
            }
        }
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path) in dirs {
        let meta = yaml(&path.join("profile.yaml"));
        let bot = &meta["ui_meta"]["factr-bots"];
        let display = bot["title"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or(meta["display_name"].as_str())
            .unwrap_or(handle(&name));
        rows.push(json!({"profile":name,"handle":handle(&name),"title":display,"description":meta["description"].as_str().unwrap_or(""),"connection_id":"local","managed":bot.is_object()}));
    }
    rows
}
fn remote(base: &Path) -> Vec<Value> {
    std::fs::read(base.join("bot_relay/roster.json"))
        .ok()
        .and_then(|s| serde_json::from_slice::<Value>(&s).ok())
        .and_then(|v| v["agents"].as_array().cloned())
        .unwrap_or_default()
}
fn resolve<'a>(raw: &str, local: &'a [Value], peers: &'a [Value]) -> Result<&'a Value> {
    let raw = raw.trim().trim_start_matches('@').to_lowercase();
    let (want, connection) = raw
        .split_once('@')
        .map_or((raw.as_str(), None), |(a, b)| (a, Some(b)));
    let id_match = |r: &&Value| {
        [r["profile"].as_str(), r["handle"].as_str()]
            .into_iter()
            .flatten()
            .any(|v| v.eq_ignore_ascii_case(want))
    };
    let alias_match = |r: &&Value| {
        aliases(r["title"].as_str().unwrap_or(""))
            .iter()
            .any(|a| a == want)
    };
    // Folder ids and reserved default handles have authority over friendly names.
    if connection.is_none() {
        if let Some(r) = local.iter().find(id_match) {
            return Ok(r);
        }
        let matches: Vec<_> = local.iter().filter(alias_match).collect();
        if matches.len() == 1 {
            return Ok(matches[0]);
        }
        if matches.len() > 1 {
            bail!("ambiguous teammate '{raw}'; use its profile name");
        }
    }
    let peers: Vec<_> = peers
        .iter()
        .filter(|r| {
            connection.is_none_or(|c| {
                r["connection_id"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(c))
            })
        })
        .collect();
    let mut matches: Vec<_> = peers.iter().copied().filter(id_match).collect();
    if matches.is_empty() {
        matches = peers.iter().copied().filter(alias_match).collect();
    }
    match matches.as_slice() {
        [r] => Ok(r),
        [] => bail!(
            "No teammate named '{raw}'. Teammates: {}",
            local
                .iter()
                .map(|r| format!("@{}", r["handle"].as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => bail!("ambiguous teammate '{raw}'; use handle@connection"),
    }
}
fn atomic(path: &Path, value: &Value) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("missing parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", rand::random::<u64>()));
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
fn queue(base: &Path, sender: &str, session: &str, args: &Value) -> Result<Value> {
    let message = args["message"].as_str().unwrap_or("").trim();
    anyhow::ensure!(!message.is_empty(), "message is required");
    anyhow::ensure!(
        message.chars().count() <= 16_000,
        "message too long (maximum 16000 characters)"
    );
    let local = roster(base);
    let peers = remote(base);
    anyhow::ensure!(
        local.iter().any(|r| r["managed"] == true) || !peers.is_empty(),
        "This install has no bot roster"
    );
    let target = resolve(args["target"].as_str().unwrap_or(""), &local, &peers)?;
    anyhow::ensure!(
        target["profile"] != sender || target["connection_id"] != "local",
        "You can't message yourself. Pick a teammate"
    );
    let fresh = std::fs::metadata(base.join("bot_relay/roster.json"))
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(600));
    anyhow::ensure!(
        !fresh || target["online"] != false,
        "[reason: runtime_offline] that teammate is offline"
    );
    let hops_file = base.join("bot_relay/hops").join(format!("{sender}.json"));
    let hops = std::fs::read(hops_file)
        .ok()
        .and_then(|s| serde_json::from_slice::<Value>(&s).ok())
        .filter(|v| chrono::Utc::now().timestamp() - v["at"].as_i64().unwrap_or(0) <= 1200)
        .and_then(|v| v["hops"].as_u64())
        .unwrap_or(0)
        + 1;
    anyhow::ensure!(
        hops <= 2,
        "[reason: hop_limit] this message already crossed 2 bots; it was not queued"
    );
    let name = local
        .iter()
        .find(|r| r["profile"] == sender)
        .and_then(|r| r["title"].as_str())
        .unwrap_or(handle(sender));
    let id = format!("{:032x}", rand::random::<u128>());
    let envelope = json!({"id":id,"created_at":chrono::Utc::now().timestamp(),"hops":hops,"from_profile":sender,"from_handle":handle(sender),
        "target_connection":target["connection_id"],"target_profile":target["profile"],"target_handle":target["handle"],
        "message":format!("Message from 🤖 {name} (@{}): {message}",handle(sender)),"sender_session":session,"native":true});
    atomic(
        &base.join("bot_relay/native").join(format!("{id}.json")),
        &envelope,
    )?;
    atomic(
        &base.join("bot_relay/outbox").join(format!("{id}.json")),
        &envelope,
    )?;
    Ok(json!({"status":"queued","delivery_id":id,"to":format!("@{}",target["handle"].as_str().unwrap_or("")),"reply_delivery":"notification","detail":"Finish your turn now. The reply or delivery failure wakes this chat later; do not wait, poll or resend."}))
}

pub(super) fn reminder(previous: Option<&str>) -> Option<String> {
    let local = roster(&root());
    if !local.iter().any(|r| r["managed"] == true) { return previous.map(str::to_string); }
    let rows = local.into_iter().chain(remote(&root())).filter(|r| r["profile"] != own() || r["connection_id"] != "local").map(|r| format!("- @{}{}: {} {}",r["handle"].as_str().unwrap_or(""), if r["connection_id"] != "local" { format!("@{}",r["connection_id"].as_str().unwrap_or("")) } else {String::new()},r["title"].as_str().unwrap_or(""),r["description"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n");
    Some(format!("{}\nMessaging other agents. Compose your own message and use message_agent; the later reply wakes this chat. Teammates and roles:\n{rows}",previous.unwrap_or("")))
}

impl Conn {
    pub(super) async fn prepare_bot_tool(self: &Arc<Self>, p: &Value) -> Result<(), RpcError> {
        let id = p["session_id"].as_str().unwrap_or_default();
        let sender = own();
        let local = roster(&root());
        let is_chat = sender == "default"
            || factr_base::session::Session::load(id).is_ok_and(|s| {
                s.custom_title.as_deref().or(s.title.as_deref()) == Some("Bot Chat")
            });
        if is_chat
            && self.run_kind != "cron"
            && (local.iter().any(|r| r["managed"] == true) || !remote(&root()).is_empty())
        {
            self.ensure_attached(id).await.map_err(RpcError::internal)?;
            let schema: Value = serde_json::from_str(include_str!("../../tests/fixtures/message_agent.schema.json")).expect("upstream message_agent schema");
            let request = json!({"req":"configure_tools","session_id":id,"tools":{"disabled":["mcp__factr_teammates__message_agent"],"custom":[schema["function"].clone()]}});
            let mut configured = self.call(request.clone()).await;
            for _ in 0..20 {
                if !configured
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains("Session is busy"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                configured = self.call(request.clone()).await;
            }
            if let Err(err) = configured {
                // A second attachment keeps the existing callback owner's handler.
                if !err.to_string().contains("owned by another client")
                    && !err.to_string().contains("Session is busy")
                {
                    return Err(RpcError::internal(err));
                }
            }
        }
        Ok(())
    }

    pub(super) async fn bot_reply(self: &Arc<Self>, p: &Value) -> Result<Value, RpcError> {
        let id = p["id"]
            .as_str()
            .filter(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| RpcError::params("invalid delivery id"))?;
        let file = root().join("bot_relay/native").join(format!("{id}.json"));
        let envelope: Value = std::fs::read(&file)
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .ok_or_else(|| RpcError::params("unknown delivery id"))?;
        if envelope["from_profile"] != own() {
            return Err(RpcError::params("reply reached the wrong sender engine"));
        }
        let session = envelope["sender_session"]
            .as_str()
            .ok_or_else(|| RpcError::params("missing sender session"))?;
        let reply = p["reply"].as_str().unwrap_or("");
        let error = p["error"].as_str().unwrap_or("");
        if error.is_empty() && (reply.trim().is_empty() || bot_chat_deliver::is_silence(reply)) {
            return Ok(json!({"ok":true,"silent":true}));
        }
        let target = envelope["target_handle"].as_str().unwrap_or("teammate");
        let text = if error.is_empty() {
            format!(
                "Reply from @{target}:\n{}",
                bot_chat_deliver::cap_reply(reply)
            )
        } else {
            format!(
                "Delivery to @{target} failed [reason: {}]: {error}",
                p["reason"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unknown")
            )
        };
        self.deliver_rpc(
            &json!({"text":text,"envelope_id":format!("reply-{id}"),"hops":envelope["hops"]}),
            Some(session.to_string()),
        )
        .await
    }

    pub(super) fn bot_tool(self: &Arc<Self>, frame: &Value) -> bool {
        if frame["ev"] != "tool_call" || frame["name"] != "message_agent" {
            return false;
        }
        let (Some(session), Some(call)) = (frame["session_id"].as_str(), frame["call_id"].as_str())
        else {
            return false;
        };
        let (session, call, input, conn) = (
            session.to_string(),
            call.to_string(),
            frame["input"].clone(),
            self.clone(),
        );
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
            let queued = queue(&root(),&own(),&session,&input);
            let (output,error) = match queued { Ok(v)=>(v.to_string(),None),Err(e)=>(e.to_string(),Some(e.to_string())) };
            let _ = conn.call(json!({"req":"tool_result","session_id":session,"call_id":call,"output":output,"error":error})).await;
        })
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_profile_wins_and_ambiguous_friendly_names_are_refused() {
        let rows = vec![
            json!({"profile":"a","handle":"a","title":"Shared","connection_id":"local"}),
            json!({"profile":"b","handle":"b","title":"Shared","connection_id":"local"}),
        ];
        assert_eq!(resolve("a", &rows, &[]).unwrap()["profile"], "a");
        assert!(resolve("shared", &rows, &[]).is_err());
        assert!(resolve("missing", &rows, &[]).is_err());
    }
    #[test]
    fn queued_envelope_matches_the_python_relay_contract() {
        let base = std::env::temp_dir().join(format!("bots-native-{:x}", rand::random::<u64>()));
        for name in ["a", "b"] {
            let dir = base.join("profiles").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("profile.yaml"),
                format!("ui_meta:\n  factr-bots:\n    title: {name}\n"),
            )
            .unwrap();
        }
        let ack = queue(
            &base,
            "a",
            "sender-chat",
            &json!({"target":"b","message":"hi"}),
        )
        .unwrap();
        assert_eq!(ack["status"], "queued");
        let envelope: Value = serde_json::from_slice(
            &std::fs::read(
                base.join("bot_relay/outbox")
                    .join(format!("{}.json", ack["delivery_id"].as_str().unwrap())),
            )
            .unwrap(),
        )
        .unwrap();
        for (key, value) in [
            ("from_profile", json!("a")),
            ("from_handle", json!("a")),
            ("target_connection", json!("local")),
            ("target_profile", json!("b")),
            ("target_handle", json!("b")),
            ("message", json!("Message from 🤖 a (@a): hi")),
            ("hops", json!(1)),
        ] {
            assert_eq!(envelope[key], value, "{key}");
        }
        let mut comparable = envelope.clone();
        comparable.as_object_mut().unwrap().remove("native");
        comparable.as_object_mut().unwrap().remove("sender_session");
        comparable["id"] = json!("<id>");
        comparable["created_at"] = json!(0);
        let golden: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/bot_envelope.json")).unwrap();
        assert_eq!(
            comparable, golden,
            "Python enqueue_envelope golden, captured in an isolated home"
        );
        assert!(queue(&base, "a", "s", &json!({"target":"a","message":"no"})).is_err());
        assert!(
            queue(
                &base,
                "a",
                "s",
                &json!({"target":"b","message":"x".repeat(16001)})
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
