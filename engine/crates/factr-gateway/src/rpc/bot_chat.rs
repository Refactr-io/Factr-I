//! `bot.chat.get` / `bot.chat.get_many` / `bot.chat.ensure` / `bot.chat.claim`: the canonical "Bot Chat".
//!
//! A bot runs on its own engine, so its chats live in this engine's store, not in the primary's
//! state.db that `profiles.list` reads for `canonical_session`. The window asks the bot's own engine
//! instead. Host engines of several bots share one engine home (`FACTR_HOME`, one session store), so a
//! title alone cannot say whose "Bot Chat" it is: the registry is a pointer per profile
//! (`bot_chat:<profile>` in the engine's settings) to a live, top-level session. `ensure` mints the chat
//! the way the desktop does (`session.create` with the title) when the pointer is empty or stale, and
//! `claim` lets the desktop's own create flow register the session it just made. One minter at a time,
//! so concurrent callers still end up with exactly one chat.
//!
//! Every call may name a `profile`; the plugin always does. Without one a call means the engine's own profile. `get` and
//! `get_many` take `profile`(s): host engines share the pointer table, so the primary
//! engine answers for every bot on the host without any bot engine being started (the roster uses this).

use super::*;

const TITLE: &str = "Bot Chat";

pub(super) fn handles(method: &str) -> bool {
    matches!(method, "bot.chat.get" | "bot.chat.get_many" | "bot.chat.ensure" | "bot.chat.claim")
}

/// The pointed-at session among the engine's session infos: live and top-level.
fn find<'a>(infos: &'a [Value], id: &str) -> Option<&'a Value> {
    infos.iter().find(|s| s["session_id"] == id && s["parent_session_id"].is_null() && s["archived"] != true)
}

/// The row as the roster's `canonical_session` carries it.
fn canonical_row(info: &Value) -> Value {
    let mut row = map::session_row(info);
    row["root_title"] = json!(TITLE);
    row["last_active"] = row["started_at"].clone();
    // The window waits to paint as many messages as this says (`expectHistory`). `session_row` also counts the
    // internal system-reminder turns a new chat starts with, which the transcript hides: a brand-new chat then
    // reported 1, showed 0, and its open waited forever. Count what the transcript shows.
    if let Some(stored) = row["id"].as_str().and_then(|id| factr_base::session::Session::load(id).ok()) {
        row["message_count"] = json!(visible_messages(&stored));
    }
    row
}

/// User and assistant messages the transcript shows (no internal system reminders).
fn visible_messages(session: &factr_base::session::Session) -> usize {
    session
        .messages
        .iter()
        .filter(|m| matches!(m.role, factr_base::message::Role::User | factr_base::message::Role::Assistant))
        .filter(|m| !factr_base::session::is_internal_system_reminder_message(m))
        .count()
}

/// One minter at a time per engine process: two windows (or a window and a relay) asking at once
/// must not both find nothing and both create.
static MINT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What the registry needs from the engine: the pointer, the session list, a way to mint.
trait Store {
    /// The registered session id of `profile` (the engine's own profile when `None`).
    fn pointer(&self, profile: Option<&str>) -> Option<String>;
    fn set_pointer(&self, profile: Option<&str>, id: &str);
    /// The profile this engine serves (what a call without `profile` means).
    fn own_profile(&self) -> String;
    async fn list(&self) -> Result<Vec<Value>, RpcError>;
    /// Create a session titled "Bot Chat"; its id.
    async fn create(&self) -> Result<String, RpcError>;
}

/// The pointer's live session, if any.
async fn current(store: &impl Store, profile: Option<&str>) -> Result<Option<Value>, RpcError> {
    let Some(id) = store.pointer(profile) else { return Ok(None) };
    Ok(find(&store.list().await?, &id).cloned())
}

/// `get_many`: each profile's row (or null), from one session listing. Read-only, never creates.
async fn resolve_many(store: &impl Store, profiles: &[&str]) -> Result<Value, RpcError> {
    let infos = store.list().await?;
    let mut chats = serde_json::Map::new();
    for profile in profiles {
        let row = store.pointer(Some(profile)).and_then(|id| find(&infos, &id).map(canonical_row));
        chats.insert((*profile).to_string(), row.unwrap_or(Value::Null));
    }
    Ok(json!({ "chats": chats }))
}

/// `get`: the profile's Bot Chat or none. `ensure`: the same, minted when missing. `claim`: register `id`
/// (a chat the desktop just created) unless the profile already has a live one, which then wins.
async fn resolve(store: &impl Store, method: &str, claim: Option<&str>, profile: Option<&str>) -> Result<Value, RpcError> {
    // Any engine may read or register a chat for any profile (host engines share one store, so the pointer lands
    // right whichever engine the window happened to reach). Only the profile's own engine may mint its chat.
    let profile = profile.filter(|p| !p.is_empty());
    let may_mint = method == "bot.chat.ensure" && profile.is_none_or(|p| p == store.own_profile());
    let _one = MINT.lock().await;
    if let Some(info) = current(store, profile).await? {
        return Ok(json!({ "canonical_session": canonical_row(&info), "created": false, "claimed": false }));
    }
    if let Some(id) = claim {
        let infos = store.list().await?;
        let info = find(&infos, id).ok_or_else(|| RpcError::params("unknown or archived session_id"))?;
        store.set_pointer(profile, id);
        return Ok(json!({ "canonical_session": canonical_row(info), "created": false, "claimed": true }));
    }
    if !may_mint {
        return Ok(json!({ "canonical_session": null, "created": false, "claimed": false }));
    }
    let id = store.create().await?;
    let infos = store.list().await?;
    let info = find(&infos, &id).ok_or_else(|| RpcError::internal(anyhow!("the Bot Chat was created but the engine does not list it")))?;
    store.set_pointer(profile, &id);
    Ok(json!({ "canonical_session": canonical_row(info), "created": true, "claimed": false }))
}

struct EngineStore<'a> {
    conn: &'a Arc<Conn>,
    params: &'a Value,
}

impl EngineStore<'_> {
    fn key(profile: Option<&str>) -> String {
        let own = crate::profile::current().name;
        let name = profile.or(own.as_deref()).filter(|n| !n.is_empty()).unwrap_or("default");
        format!("bot_chat:{name}")
    }
}

impl Store for EngineStore<'_> {
    fn pointer(&self, profile: Option<&str>) -> Option<String> {
        crate::rpc::entries_or_log(&self.conn.config.home)?.setting(&Self::key(profile)).filter(|id| !id.is_empty())
    }

    fn set_pointer(&self, profile: Option<&str>, id: &str) {
        if let Some(store) = crate::rpc::entries_or_log(&self.conn.config.home) {
            let _ = store.set_setting(&Self::key(profile), id);
        }
    }

    fn own_profile(&self) -> String {
        crate::profile::current().name.filter(|n| !n.is_empty()).unwrap_or_else(|| "default".into())
    }

    async fn list(&self) -> Result<Vec<Value>, RpcError> {
        let reply = self
            .conn
            .call(json!({ "req": "list_sessions", "limit": 1000, "include_archived": false }))
            .await
            .map_err(RpcError::internal)?;
        Ok(reply["sessions"].as_array().cloned().unwrap_or_default())
    }

    /// The desktop's own steps: `session.create` with the title, then the same eager `session.title`
    /// write, so the row is in the store before anyone can ask again.
    async fn create(&self) -> Result<String, RpcError> {
        let mut params = json!({ "title": TITLE, "hidden": true, "follow_profile_config": true });
        if let Some(cwd) = self.params["cwd"].as_str() {
            params["cwd"] = json!(cwd);
        }
        let created = Box::pin(self.conn.dispatch("session.create", &params)).await?;
        let id = created["stored_session_id"].as_str().or(created["session_id"].as_str()).unwrap_or_default().to_string();
        if id.is_empty() {
            return Err(RpcError::internal(anyhow!("the engine created no session")));
        }
        Box::pin(self.conn.dispatch("session.title", &json!({ "session_id": id, "title": TITLE }))).await?;
        Ok(id)
    }
}

impl Conn {
    pub(super) async fn bot_chat(self: &Arc<Self>, method: &str, p: &Value) -> Result<Value, RpcError> {
        let claim = match method {
            "bot.chat.claim" => Some(p["session_id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| RpcError::params("session_id is required"))?),
            _ => None,
        };
        let store = EngineStore { conn: self, params: p };
        if method == "bot.chat.get_many" {
            let profiles: Vec<&str> = p["profiles"].as_array().map(|l| l.iter().filter_map(Value::as_str).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
            return resolve_many(&store, &profiles).await;
        }
        resolve(&store, method, claim, p["profile"].as_str().filter(|n| !n.is_empty())).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn info(id: &str) -> Value {
        json!({ "session_id": id, "title": TITLE, "parent_session_id": null })
    }

    /// A store that lists what was created, like the engine's.
    struct Fake {
        rows: Mutex<Vec<Value>>,
        pointers: Mutex<std::collections::HashMap<String, String>>,
        creates: AtomicUsize,
    }

    impl Fake {
        fn new(rows: Vec<Value>) -> Self {
            Self { rows: Mutex::new(rows), pointers: Mutex::new(Default::default()), creates: AtomicUsize::new(0) }
        }
        async fn run(&self, method: &str) -> Value {
            resolve(self, method, None, None).await.unwrap()
        }
        fn id_of(v: &Value) -> Option<&str> {
            v["canonical_session"]["id"].as_str()
        }
    }

    impl Store for Fake {
        fn pointer(&self, profile: Option<&str>) -> Option<String> {
            self.pointers.lock().unwrap().get(profile.unwrap_or("own")).cloned()
        }
        fn set_pointer(&self, profile: Option<&str>, id: &str) {
            self.pointers.lock().unwrap().insert(profile.unwrap_or("own").into(), id.to_string());
        }
        fn own_profile(&self) -> String {
            "own".into()
        }
        async fn list(&self) -> Result<Vec<Value>, RpcError> {
            Ok(self.rows.lock().unwrap().clone())
        }
        async fn create(&self) -> Result<String, RpcError> {
            tokio::task::yield_now().await; // a slow create: a second caller must queue behind it
            let id = format!("minted-{}", self.creates.fetch_add(1, Ordering::SeqCst));
            self.rows.lock().unwrap().push(info(&id));
            Ok(id)
        }
    }

    #[test]
    fn only_a_live_top_level_session_can_be_the_registered_chat() {
        let mut archived = info("old");
        archived["archived"] = json!(true);
        let mut child = info("child");
        child["parent_session_id"] = json!("parent");
        let infos = vec![archived, child, info("mine")];
        assert!(find(&infos, "mine").is_some());
        assert!(find(&infos, "old").is_none() && find(&infos, "child").is_none() && find(&infos, "nope").is_none());
    }

    #[test]
    fn a_new_chat_reports_the_messages_the_transcript_shows_not_its_hidden_reminders() {
        use factr_base::message::{ContentBlock, Role};
        let text = |t: &str| vec![ContentBlock::Text { text: t.into(), cache_control: None }];
        let mut session = factr_base::session::Session::create(None, None);
        // The persona/system reminder a new chat starts with is a user turn the transcript hides.
        session.add_message(Role::User, text("<system-reminder>\nAs you answer, use this context.\n</system-reminder>"));
        assert_eq!(visible_messages(&session), 0, "the window would otherwise wait for history that never comes");
        session.add_message(Role::User, text("hello"));
        session.add_message(Role::Assistant, text("hi"));
        assert_eq!(visible_messages(&session), 2);
    }

    #[test]
    fn the_row_has_the_roster_shape() {
        let row = canonical_row(&info("s1"));
        for key in ["id", "resolved_id", "root_title", "title", "preview", "started_at", "last_active", "message_count"] {
            assert!(row.get(key).is_some(), "{key} missing in {row}");
        }
        assert_eq!((row["id"].as_str(), row["root_title"].as_str(), row["title"].as_str()), (Some("s1"), Some(TITLE), Some(TITLE)));
    }

    #[tokio::test]
    async fn get_never_creates_and_ensure_creates_once_then_finds_the_same_chat() {
        let store = Fake::new(Vec::new());
        let missing = store.run("bot.chat.get").await;
        assert!(missing["canonical_session"].is_null() && missing["created"] == false);
        assert_eq!(store.creates.load(Ordering::SeqCst), 0, "get is read-only");

        let first = store.run("bot.chat.ensure").await;
        assert_eq!((first["created"].as_bool(), Fake::id_of(&first)), (Some(true), Some("minted-0")));
        let again = store.run("bot.chat.ensure").await;
        assert_eq!((again["created"].as_bool(), Fake::id_of(&again)), (Some(false), Some("minted-0")));
        assert_eq!(Fake::id_of(&store.run("bot.chat.get").await), Some("minted-0"));
        assert_eq!(store.creates.load(Ordering::SeqCst), 1, "exactly one Bot Chat per profile");
    }

    #[tokio::test]
    async fn another_profiles_chat_in_a_shared_store_is_not_this_profiles_chat() {
        // The store lists a "Bot Chat" (another bot's, titled the same). With no pointer of our own it is not ours.
        let store = Fake::new(vec![info("someone-elses")]);
        assert!(store.run("bot.chat.get").await["canonical_session"].is_null());
        assert_eq!(Fake::id_of(&store.run("bot.chat.ensure").await), Some("minted-0"));
    }

    #[tokio::test]
    async fn concurrent_ensures_still_mint_exactly_one() {
        let store = std::sync::Arc::new(Fake::new(Vec::new()));
        let runs: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                tokio::spawn(async move { Fake::id_of(&store.run("bot.chat.ensure").await).unwrap().to_string() })
            })
            .collect();
        let mut ids = Vec::new();
        for run in runs {
            ids.push(run.await.unwrap());
        }
        assert!(ids.iter().all(|id| id == "minted-0"), "{ids:?}");
        assert_eq!(store.creates.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_retired_chat_is_replaced_and_a_stale_pointer_is_not_trusted() {
        let store = Fake::new(vec![info("first")]);
        store.set_pointer(None, "first");
        assert_eq!(Fake::id_of(&store.run("bot.chat.get").await), Some("first"));
        store.rows.lock().unwrap()[0]["archived"] = json!(true); // archived = retired
        assert!(store.run("bot.chat.get").await["canonical_session"].is_null());
        assert_eq!(Fake::id_of(&store.run("bot.chat.ensure").await), Some("minted-0"));
        store.rows.lock().unwrap().clear(); // the pointed-at file is gone
        assert!(store.run("bot.chat.get").await["canonical_session"].is_null());
    }

    #[tokio::test]
    async fn claim_registers_the_desktops_own_chat_unless_one_already_wins() {
        let store = Fake::new(vec![info("made-by-the-desktop"), info("racer")]);
        let claimed = resolve(&store, "bot.chat.claim", Some("made-by-the-desktop"), None).await.unwrap();
        assert_eq!((claimed["claimed"].as_bool(), Fake::id_of(&claimed)), (Some(true), Some("made-by-the-desktop")));
        let lost = resolve(&store, "bot.chat.claim", Some("racer"), None).await.unwrap();
        assert_eq!((lost["claimed"].as_bool(), Fake::id_of(&lost)), (Some(false), Some("made-by-the-desktop")), "the registered chat wins");
        let missing = resolve(&Fake::new(Vec::new()), "bot.chat.claim", Some("ghost"), None).await;
        assert!(missing.is_err(), "an unknown session cannot be registered");
    }

    #[tokio::test]
    async fn a_chat_that_never_shows_up_in_the_list_is_an_error_not_a_second_mint() {
        struct Blind(AtomicUsize);
        impl Store for Blind {
            fn pointer(&self, _: Option<&str>) -> Option<String> {
                None
            }
            fn set_pointer(&self, _: Option<&str>, _: &str) {}
            fn own_profile(&self) -> String {
                "own".into()
            }
            async fn list(&self) -> Result<Vec<Value>, RpcError> {
                Ok(Vec::new())
            }
            async fn create(&self) -> Result<String, RpcError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok("ghost".into())
            }
        }
        let blind = Blind(AtomicUsize::new(0));
        assert!(resolve(&blind, "bot.chat.ensure", None, None).await.is_err());
        assert_eq!(blind.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_primary_answers_for_other_profiles_from_the_shared_pointers_without_creating_anything() {
        let store = Fake::new(vec![info("alpha-chat"), info("beta-chat")]);
        store.pointers.lock().unwrap().extend([("alpha".to_string(), "alpha-chat".to_string()), ("beta".to_string(), "beta-chat".to_string())]);
        let one = resolve(&store, "bot.chat.get", None, Some("alpha")).await.unwrap();
        assert_eq!(Fake::id_of(&one), Some("alpha-chat"), "get with a profile reads that profile's pointer");
        assert!(resolve(&store, "bot.chat.get", None, Some("gamma")).await.unwrap()["canonical_session"].is_null());
        let many = resolve_many(&store, &["alpha", "beta", "gamma"]).await.unwrap();
        assert_eq!(many["chats"]["alpha"]["id"], "alpha-chat");
        assert_eq!(many["chats"]["beta"]["id"], "beta-chat");
        assert!(many["chats"]["gamma"].is_null(), "a profile with no registered chat is null");
        assert_eq!(store.creates.load(Ordering::SeqCst), 0, "reads never create");
        // ensure never mints for a profile other than the engine's own; it reads what is registered.
        let read = resolve(&store, "bot.chat.ensure", None, Some("alpha")).await.unwrap();
        assert_eq!((read["created"].as_bool(), Fake::id_of(&read)), (Some(false), Some("alpha-chat")));
        let none = resolve(&store, "bot.chat.ensure", None, Some("gamma")).await.unwrap();
        assert!(none["canonical_session"].is_null(), "no chat is minted for another profile");
        assert_eq!(store.creates.load(Ordering::SeqCst), 0);
        // ... and its own profile (named or not) is minted.
        let own = resolve(&store, "bot.chat.ensure", None, Some("own")).await.unwrap();
        assert_eq!((own["created"].as_bool(), Fake::id_of(&own)), (Some(true), Some("minted-0")));
    }

    #[tokio::test]
    async fn claims_land_on_the_named_profile_whichever_engine_serves_them() {
        // The window's call reached some other engine: the pointer must still be the named profile's.
        let store = Fake::new(vec![info("alpha-chat"), info("beta-chat")]);
        let a = resolve(&store, "bot.chat.claim", Some("alpha-chat"), Some("alpha")).await.unwrap();
        let b = resolve(&store, "bot.chat.claim", Some("beta-chat"), Some("beta")).await.unwrap();
        assert_eq!((a["claimed"].as_bool(), b["claimed"].as_bool()), (Some(true), Some(true)));
        assert_eq!(store.pointer(Some("alpha")).as_deref(), Some("alpha-chat"));
        assert_eq!(store.pointer(Some("beta")).as_deref(), Some("beta-chat"));
        assert!(store.pointer(None).is_none(), "the serving engine's own profile is untouched");
        let again = resolve(&store, "bot.chat.get", None, Some("beta")).await.unwrap();
        assert_eq!(Fake::id_of(&again), Some("beta-chat"));
    }

    #[test]
    fn only_the_bot_chat_methods_are_handled() {
        assert!(handles("bot.chat.get") && handles("bot.chat.get_many") && handles("bot.chat.ensure") && handles("bot.chat.claim"));
        assert!(!handles("bot.chat") && !handles("session.list"));
    }
}
