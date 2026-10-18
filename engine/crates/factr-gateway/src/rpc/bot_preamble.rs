//! Bound an interactive bot's first response without changing the agent loop or provider.
use super::*;
use std::sync::{LazyLock, Mutex as StdMutex};

const LIMIT: Duration = Duration::from_secs(60);
static WAITING: LazyLock<StdMutex<HashMap<String, (bool, tokio::sync::watch::Sender<bool>)>>> =
    LazyLock::new(Default::default);

pub(super) fn progress(frame: &Value) {
    let Some(id) = frame["session_id"].as_str() else {
        return;
    };
    let mut waiting = WAITING.lock().unwrap_or_else(|e| e.into_inner());
    let Some((accepted, tx)) = waiting.get_mut(id) else {
        return;
    };
    if frame["ev"] == "message_accepted" {
        *accepted = true;
    }
    if *accepted
        && matches!(
            frame["ev"].as_str(),
            Some("text_delta" | "reasoning_delta" | "tool_start" | "turn_done" | "error")
        )
    {
        tx.send_replace(true);
    }
}

pub(super) fn trace(action: &str, id: &str, client: u64) {
    if std::env::var_os("FACTR_BOT_TRACE").is_some() {
        eprintln!("bot-attachment: {action} session={id} connection={client}");
    }
}

fn first_bot_turn(session: &factr_base::session::Session, kind: &str) -> bool {
    // Headless bot delivery already has a turn deadline and can wait behind a serial local model.
    if kind == "bot" {
        return false;
    }
    session.custom_title.as_deref().or(session.title.as_deref()) == Some("Bot Chat")
        && !session.messages.iter().any(|m| {
            m.role == factr_base::message::Role::User
                && !factr_base::session::is_internal_system_reminder_message(m)
        })
}

async fn wait_for_progress(rx: &mut tokio::sync::watch::Receiver<bool>, limit: Duration) -> bool {
    tokio::time::timeout(limit, async {
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false)
}

pub(super) struct Preamble(Option<String>);
impl Drop for Preamble {
    fn drop(&mut self) {
        if let Some(id) = &self.0 {
            let mut waiting = WAITING.lock().unwrap_or_else(|e| e.into_inner());
            if waiting.get(id).is_some_and(|(accepted, _)| !accepted) {
                if let Some((_, tx)) = waiting.remove(id) {
                    tx.send_replace(true);
                }
            }
        }
    }
}

pub(super) fn arm(conn: &Arc<Conn>, p: &Value) -> Preamble {
    let id = p["session_id"].as_str().unwrap_or_default().to_string();
    let first = std::env::var("FACTR_PROFILE").is_ok_and(|n| n != "default")
        && factr_base::session::Session::load(&id).is_ok_and(|s| first_bot_turn(&s, conn.run_kind));
    if !first {
        return Preamble(None);
    }
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    WAITING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), (false, tx));
    let (model, provider) = crate::profile::effective_default(&conn.config);
    let endpoint = std::env::var("FACTR_OPENAI_COMPAT_API_BASE")
        .ok()
        .and_then(|s| {
            let mut url = reqwest::Url::parse(&s).ok()?;
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            Some(url.to_string())
        });
    eprintln!(
        "bot-preamble: submit session={id} connection={} provider={provider} model={model} endpoint={endpoint:?}",
        conn.client.id
    );
    let watched = id.clone();
    let conn = conn.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        runtime.block_on(async move {
            if !wait_for_progress(&mut rx, LIMIT).await {
                conn.bot_preamble_failed(&watched).await;
            }
            WAITING
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&watched);
        })
    });
    Preamble(Some(id))
}

impl Conn {
    async fn bot_preamble_failed(self: &Arc<Self>, id: &str) {
        eprintln!(
            "bot-preamble: timeout session={id} connection={}",
            self.client.id
        );
        // Cancellation goes through the same Stop request and persistence path as the UI.
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            self.call(json!({"req":"cancel","session_id":id})),
        )
        .await;
        self.hub.broadcast_text(json!({"jsonrpc":"2.0","method":"event","params":{
            "type":"error","session_id":id,"payload":{"message":"The bot did not start its first response within 60 seconds. Try again.","reason":"preamble_timeout"}
        }}).to_string()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn no_first_output_times_out_but_progress_finishes_immediately() {
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        assert!(!wait_for_progress(&mut rx, Duration::from_millis(5)).await);
        tx.send_replace(true);
        assert!(wait_for_progress(&mut rx, Duration::from_millis(5)).await);
    }
    #[test]
    fn only_an_empty_bot_chat_gets_the_first_response_deadline() {
        let mut s = factr_base::session::Session::create(None, Some("Bot Chat".into()));
        assert!(first_bot_turn(&s, "desktop"));
        assert!(!first_bot_turn(&s, "bot"));
        s.add_message(
            factr_base::message::Role::User,
            vec![factr_base::message::ContentBlock::Text {
                text: "hello".into(),
                cache_control: None,
            }],
        );
        assert!(!first_bot_turn(&s, "desktop"));
    }
}
