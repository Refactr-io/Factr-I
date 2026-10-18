//! Pool retirement admission. A session-list snapshot alone never grants permission to stop.
use super::*;
use std::sync::Mutex as StdMutex;
use std::time::Instant;

#[derive(Default)]
struct State {
    permit: Option<(String, bool, Instant)>,
    requests: usize,
}
static STATE: StdMutex<State> = StdMutex::new(State {
    permit: None,
    requests: 0,
});

fn expire(s: &mut State) {
    if s.permit
        .as_ref()
        .is_some_and(|(_, committed, at)| !committed && at.elapsed() > Duration::from_secs(30))
    {
        s.permit = None;
    }
}

pub(super) fn admission(method: &str) -> Result<Option<Admission>, RpcError> {
    if matches!(
        method,
        "session.create"
            | "prompt.submit"
            | "session.retry"
            | "prompt.background"
            | "prompt.btw"
            | "bot.chat.deliver"
            | "bot.chat.reply"
    ) {
        admit().map(Some)
    } else {
        Ok(None)
    }
}

pub(super) struct Admission;
pub(super) fn admit() -> Result<Admission, RpcError> {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    expire(&mut s);
    if s.permit.is_some() {
        return Err(RpcError {
            code: 409,
            message: "engine is retiring".into(),
            data: Some(json!({"reason":"retiring","retryable":true})),
        });
    }
    s.requests += 1;
    Ok(Admission)
}
impl Drop for Admission {
    fn drop(&mut self) {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.requests -= 1;
    }
}

impl Conn {
    async fn running_sessions(self: &Arc<Self>) -> Result<Vec<String>> {
        let reply = self
            .call(json!({"req":"list_sessions","include_archived":true}))
            .await?;
        let rows = reply["sessions"]
            .as_array()
            .ok_or_else(|| anyhow!("engine returned no session list"))?;
        Ok(rows
            .iter()
            .filter_map(|s| s["session_id"].as_str())
            .filter(|id| factr_app_core::turn_cancel_registry::has_active_turn(id))
            .map(str::to_string)
            .collect())
    }

    async fn pool_retirement(self: &Arc<Self>, p: &Value) -> Result<Value> {
        let action = p["action"].as_str().unwrap_or_default();
        if action == "prepare" {
            let token = {
                let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
                expire(&mut s);
                if let Some((token, _, _)) = &s.permit {
                    return Ok(json!({"ok":true,"idle":true,"token":token}));
                }
                if s.requests > 0 {
                    return Ok(json!({"ok":true,"idle":false,"requests":s.requests}));
                }
                let token = format!("{:032x}", rand::random::<u128>());
                s.permit = Some((token.clone(), false, Instant::now()));
                token
            };
            // Admission is frozen BEFORE this observation, including submits from other WebSockets.
            match self.running_sessions().await {
                Ok(running) if running.is_empty() => {
                    return Ok(json!({"ok":true,"idle":true,"token":token}));
                }
                _ => {
                    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if s.permit.as_ref().is_some_and(|(t, _, _)| *t == token) {
                        s.permit = None;
                    }
                    return Ok(json!({"ok":true,"idle":false}));
                }
            }
        }
        let token = p["token"].as_str().unwrap_or_default();
        {
            let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
            expire(&mut s);
            let Some((held, committed, _)) = &mut s.permit else {
                return Ok(json!({"ok":false}));
            };
            if token.is_empty() || held != token {
                return Ok(json!({"ok":false}));
            }
            match action {
                "cancel" if !*committed => {
                    s.permit = None;
                    return Ok(json!({"ok":true}));
                }
                "commit" => {
                    *committed = true;
                }
                "status" | "cancel_turns" => {}
                _ => return Ok(json!({"ok":false})),
            }
        }
        // A committed permit keeps admission closed. Only a real process exit frees the pool slot.
        let running = self.running_sessions().await?;
        if action == "cancel_turns" {
            for id in &running {
                let _ = self.call(json!({"req":"cancel","session_id":id})).await;
            }
        }
        Ok(json!({"ok":true,"state":"draining","idle":running.is_empty(),"running":running}))
    }
}

pub(crate) async fn request(
    config: Arc<Config>,
    hub: Arc<Hub>,
    observer: Arc<Observer>,
    p: &Value,
) -> Result<Value> {
    let (to_ws, mut rx) = mpsc::channel::<Message>(1024);
    let client = Arc::new(Client {
        id: hub.next_client_id(),
        to_ws: to_ws.clone(),
        sessions: Mutex::new(Default::default()),
    });
    let conn = Conn::new(
        config, to_ws, hub, client, observer, false, "bot", None, None,
    );
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let result = match conn.ensure_control().await { Ok(()) => conn.pool_retirement(p).await, Err(err) => Err(err) };
    end_run(&conn, None).await;
    drain.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_and_committed_permits_freeze_admission_and_only_uncommitted_ones_expire() {
        let mut s = State::default();
        s.permit = Some((
            "permit".into(),
            false,
            Instant::now() - Duration::from_secs(31),
        ));
        expire(&mut s);
        assert!(s.permit.is_none());
        s.permit = Some((
            "permit".into(),
            true,
            Instant::now() - Duration::from_secs(31),
        ));
        expire(&mut s);
        assert!(s.permit.is_some());
    }
}
