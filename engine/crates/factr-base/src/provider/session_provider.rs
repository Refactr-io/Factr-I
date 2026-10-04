//! Which model a background call runs on: the session's own.
//!
//! Memory extraction, the learning gate and refine, and the REPL's `llm_query` sub-calls are
//! "auxiliary" calls. Unless the user configured an auxiliary model (Factr `auxiliary.<task>`,
//! `agents.memory_model`, `agents.repl_sub_model`) they must run on the model the chat is on, never
//! on the model the engine started with or a built-in default. Each live agent registers its
//! provider here under its session id; an aux call forks that provider (so the chat's own model is
//! never switched). A session with no live agent (a chat closed through the gateway) is rebuilt
//! from its saved model and route on a fork of the startup provider.

use super::{MultiProvider, Provider};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, Weak};

static SESSION_PROVIDERS: LazyLock<Mutex<HashMap<String, Weak<dyn Provider>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

tokio::task_local! {
    static AUX_SESSION: String;
    static AUX_CONSUMER: crate::factr_config::AuxConsumer;
}

/// Record `provider` as the one serving `session_id` (held weakly: a dropped agent frees itself).
pub fn register_session_provider(session_id: &str, provider: &Arc<dyn Provider>) {
    let mut map = SESSION_PROVIDERS.lock().unwrap_or_else(|p| p.into_inner());
    if map.len() > 256 {
        map.retain(|_, weak| weak.strong_count() > 0);
    }
    map.insert(session_id.to_string(), Arc::downgrade(provider));
}

fn live_session_provider(session_id: &str) -> Option<Arc<dyn Provider>> {
    SESSION_PROVIDERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(session_id)
        .and_then(Weak::upgrade)
}

/// Whether `fork` is on `expected`: a fork that quietly stayed on another model (a switch the
/// provider refused) must never serve a background call, so every fork is checked.
fn on_model(fork: &dyn Provider, expected: &str) -> bool {
    fork.model().trim().eq_ignore_ascii_case(expected.trim())
}

/// A fork of the provider serving `session_id`, on the session's current model. Without a live
/// agent: a fork of the startup provider switched to the session's saved model. `None` (and an
/// error in the log) when the fork is not verifiably on that model: the call then fails visibly
/// instead of running on a hidden one.
pub fn session_provider_fork(session_id: &str) -> Option<Arc<dyn Provider>> {
    if let Some(live) = live_session_provider(session_id) {
        let expected = live.model();
        let fork = live.fork();
        if !on_model(fork.as_ref(), &expected) {
            crate::logging::error(&format!(
                "aux call for {session_id}: the fork is on {} not the chat model {expected}; refusing to use it",
                fork.model()
            ));
            return None;
        }
        return Some(fork);
    }
    let fork = super::active_provider_fork()?;
    let session = match crate::session::Session::load(session_id) {
        Ok(session) => session,
        Err(error) => {
            crate::logging::error(&format!("aux call for {session_id}: the saved session cannot be read ({error}); refusing to use the startup model"));
            return None;
        }
    };
    let Some(model) = session.model.as_deref().filter(|m| !m.trim().is_empty()) else {
        crate::logging::error(&format!("aux call for {session_id}: the session has no saved model; refusing to use the startup model"));
        return None;
    };
    let request = MultiProvider::model_switch_request_for_session_route(model, session.provider_key.as_deref(), None);
    if let Err(error) = fork.set_model(&request) {
        crate::logging::error(&format!("aux call for {session_id}: cannot use the session model {request}: {error}"));
        return None;
    }
    if !on_model(fork.as_ref(), model) {
        crate::logging::error(&format!("aux call for {session_id}: the fork is on {} not the session model {model}; refusing to use it", fork.model()));
        return None;
    }
    Some(fork)
}

/// Run `fut` (a background model call) on behalf of `session_id`; [`session_base`] sees it.
pub async fn with_aux_session<F: Future>(session_id: &str, fut: F) -> F::Output {
    AUX_SESSION.scope(session_id.to_string(), fut).await
}

/// Run `fut` as the aux consumer `consumer` (which `auxiliary.*` override applies); [`aux_provider`] sees it.
pub async fn with_aux_consumer<F: Future>(consumer: crate::factr_config::AuxConsumer, fut: F) -> F::Output {
    AUX_CONSUMER.scope(consumer, fut).await
}

/// The consumer of the current aux call, if one was named.
pub fn aux_consumer() -> Option<crate::factr_config::AuxConsumer> {
    AUX_CONSUMER.try_with(|c| *c).ok()
}

/// The provider a background model call (the gateway's `Complete`) runs on: inside [`with_aux_session`]
/// the chat's own model, else the startup provider, which must be on `startup_model` (the model the
/// engine was asked to serve). A startup provider that is not (a boot-time switch that failed for lack
/// of a key or a catalog entry) is switched again now and refused when it still is not, so a call never
/// runs on a catalog default nobody picked. The consumer ([`with_aux_consumer`], default Learning)
/// decides which `auxiliary.*` override applies to the fork.
pub fn aux_provider(startup: &Arc<dyn Provider>, startup_model: &str) -> anyhow::Result<Arc<dyn Provider>> {
    use crate::factr_config::{AuxConsumer, provider_for};
    let base = match AUX_SESSION.try_with(|id| session_provider_fork(id)) {
        Ok(fork) => fork.ok_or_else(|| anyhow::anyhow!("the chat's model could not be resolved for this background call"))?,
        Err(_) => {
            if !on_model(startup.as_ref(), startup_model) {
                // Re-apply the model the engine was asked to serve (a key may have arrived since boot).
                let _ = startup.set_model(startup_model);
            }
            if !on_model(startup.as_ref(), startup_model) {
                anyhow::bail!(
                    "the engine is on {} but {startup_model} was chosen; refusing a background call on a model nobody picked",
                    startup.model()
                );
            }
            startup.clone()
        }
    };
    Ok(provider_for(aux_consumer().unwrap_or(AuxConsumer::Learning), base))
}

/// The provider an aux call should start from: inside [`with_aux_session`] the session's own
/// (live, else rebuilt from its saved model), and `None` when that cannot be verified, so the
/// call fails instead of using the startup model. Outside the scope: `fallback`.
pub fn session_base(fallback: Arc<dyn Provider>) -> Option<Arc<dyn Provider>> {
    match AUX_SESSION.try_with(|id| session_provider_fork(id)) {
        Ok(fork) => fork,
        Err(_) => Some(fallback),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::message::{Message, ToolDefinition};
    use crate::provider::EventStream;
    use factr_provider_core::{SimpleCompletion, SimpleUsage};
    use std::sync::RwLock;

    /// A provider that answers every one-shot call with the model it is on, and can switch model.
    pub(crate) struct ModelEcho {
        model: RwLock<String>,
    }

    impl ModelEcho {
        pub(crate) fn on(model: &str) -> Arc<dyn Provider> {
            Arc::new(Self { model: RwLock::new(model.to_string()) })
        }
    }

    #[async_trait::async_trait]
    impl Provider for ModelEcho {
        async fn complete(&self, _: &[Message], _: &[ToolDefinition], _: &str, _: Option<&str>) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
        fn name(&self) -> &str {
            "model-echo"
        }
        fn model(&self) -> String {
            self.model.read().unwrap().clone()
        }
        fn set_model(&self, model: &str) -> anyhow::Result<()> {
            *self.model.write().unwrap() = model.to_string();
            Ok(())
        }
        fn fork(&self) -> Arc<dyn Provider> {
            ModelEcho::on(&self.model())
        }
        async fn complete_simple_with_usage(&self, _: &str, _: &str) -> anyhow::Result<SimpleCompletion> {
            Ok(SimpleCompletion { text: self.model(), usage: Some(SimpleUsage { input: 1, output: 1, cache_read: 0, cache_write: 0 }) })
        }
    }

    #[test]
    fn an_aux_call_forks_the_session_provider_and_never_switches_it() {
        let session: Arc<dyn Provider> = ModelEcho::on("chat-model");
        register_session_provider("sp-fork", &session);
        let fork = session_provider_fork("sp-fork").expect("registered");
        assert_eq!(fork.model(), "chat-model");
        fork.set_model("something-else").unwrap();
        assert_eq!(session.model(), "chat-model", "the chat's own model is untouched");
        // the chat switches model: the next aux call follows it
        session.set_model("picked-later").unwrap();
        assert_eq!(session_provider_fork("sp-fork").unwrap().model(), "picked-later");
    }

    /// A provider whose forks silently refuse every model switch and stay on `stuck`
    /// (what an OpenAI fork does for a model missing from the cached catalog).
    struct Sticky {
        stuck: String,
        current: RwLock<String>,
    }

    #[async_trait::async_trait]
    impl Provider for Sticky {
        async fn complete(&self, _: &[Message], _: &[ToolDefinition], _: &str, _: Option<&str>) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
        fn name(&self) -> &str {
            "sticky"
        }
        fn model(&self) -> String {
            self.current.read().unwrap().clone()
        }
        fn set_model(&self, model: &str) -> anyhow::Result<()> {
            anyhow::bail!("unsupported model {model}")
        }
        fn fork(&self) -> Arc<dyn Provider> {
            Arc::new(Sticky { stuck: self.stuck.clone(), current: RwLock::new(self.stuck.clone()) })
        }
    }

    fn save_session(id: &str, model: &str) {
        let mut session = crate::session::Session::create_with_id(id.to_string(), None, None);
        session.model = Some(model.to_string());
        session.save_prepared().unwrap();
    }

    #[test]
    fn a_fork_that_stayed_on_another_model_is_refused_not_used() {
        // The chat is on luna; its fork came back on the provider default.
        let chat: Arc<dyn Provider> = Arc::new(Sticky { stuck: "gpt-6-astra".into(), current: RwLock::new("gpt-5.6-luna".into()) });
        register_session_provider("sp-sticky", &chat);
        assert!(session_provider_fork("sp-sticky").is_none(), "never a hidden second model");
    }

    #[test]
    fn a_session_with_no_live_agent_is_rebuilt_from_its_saved_model_or_refused() {
        let _env = crate::storage::lock_test_env();
        super::super::set_active_provider(ModelEcho::on("startup-model"));
        save_session("sp-saved", "saved-model");
        assert_eq!(session_provider_fork("sp-saved").expect("saved model").model(), "saved-model");
        // dropped agent: same answer from disk
        {
            let session: Arc<dyn Provider> = ModelEcho::on("chat-model");
            register_session_provider("sp-saved", &session);
        }
        assert_eq!(session_provider_fork("sp-saved").unwrap().model(), "saved-model");
        // an unreadable session never falls back to the startup model
        assert!(session_provider_fork("sp-never-saved").is_none());
        // a startup provider that refuses the saved model is refused too
        super::super::set_active_provider(Arc::new(Sticky { stuck: "gpt-6-astra".into(), current: RwLock::new("gpt-6-astra".into()) }));
        assert!(session_provider_fork("sp-saved").is_none());
        super::super::set_active_provider(ModelEcho::on("startup-model"));
    }

    #[tokio::test]
    async fn session_base_follows_the_session_inside_the_scope_and_the_fallback_outside() {
        let _env = crate::storage::lock_test_env();
        let session: Arc<dyn Provider> = ModelEcho::on("chat-model");
        register_session_provider("sp-base", &session);
        super::super::set_active_provider(ModelEcho::on("startup-model"));
        let startup = ModelEcho::on("startup-model");
        assert_eq!(session_base(startup.clone()).unwrap().model(), "startup-model");
        let inside = with_aux_session("sp-base", async { session_base(startup.clone()).unwrap().model() }).await;
        assert_eq!(inside, "chat-model");
        // the agent is gone: the saved session still decides, never the startup model
        save_session("sp-gone", "saved-model");
        let gone = with_aux_session("sp-gone", async { session_base(startup.clone()).unwrap().model() }).await;
        assert_eq!(gone, "saved-model");
        let unknown = with_aux_session("sp-unknown", async { session_base(startup.clone()).is_none() }).await;
        assert!(unknown, "an unknown session fails the call instead of using the startup provider");
    }

    #[test]
    fn fork_on_same_model_errors_when_the_fork_could_not_stay_on_the_model() {
        let chat: Arc<dyn Provider> = Arc::new(Sticky { stuck: "gpt-6-astra".into(), current: RwLock::new("gpt-5.6-luna".into()) });
        let err = chat.fork_on_same_model().err().expect("refused");
        assert!(err.to_string().contains("gpt-6-astra") && err.to_string().contains("gpt-5.6-luna"), "{err}");
        let fine: Arc<dyn Provider> = ModelEcho::on("gpt-5.6-luna");
        assert_eq!(fine.fork_on_same_model().unwrap().model(), "gpt-5.6-luna");
    }

    #[tokio::test]
    async fn the_startup_provider_is_refused_when_it_is_not_on_the_model_that_was_chosen() {
        // Boot could not switch to luna (no key yet): the provider is on the catalog default.
        let startup: Arc<dyn Provider> = Arc::new(Sticky { stuck: "gpt-6-astra".into(), current: RwLock::new("gpt-6-astra".into()) });
        let err = aux_provider(&startup, "gpt-5.6-luna").err().expect("refused outside a chat");
        assert!(err.to_string().contains("gpt-6-astra") && err.to_string().contains("gpt-5.6-luna"), "{err}");
        // A later key lets the switch succeed: the call re-applies it and runs on the chosen model.
        let later: Arc<dyn Provider> = ModelEcho::on("gpt-6-astra");
        assert_eq!(aux_provider(&later, "gpt-5.6-luna").unwrap().model(), "gpt-5.6-luna");
        assert_eq!(later.model(), "gpt-5.6-luna");
        // On the chosen model already: used as is.
        let ok: Arc<dyn Provider> = ModelEcho::on("gpt-5.6-luna");
        assert_eq!(aux_provider(&ok, "gpt-5.6-luna").unwrap().model(), "gpt-5.6-luna");
    }

    #[tokio::test]
    async fn a_one_shot_follows_the_chat_model_and_takes_no_learning_override() {
        let _env = crate::storage::lock_test_env();
        let dir = std::env::temp_dir().join(format!("one-shot-aux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), "auxiliary:\n  background_review:\n    provider: anthropic\n    model: claude-haiku\n").unwrap();
        let before = std::env::var_os("FACTR_CONFIG_HOME");
        crate::env::set_var("FACTR_CONFIG_HOME", &dir);
        let chat: Arc<dyn Provider> = ModelEcho::on("chat-model");
        register_session_provider("sp-one-shot", &chat);
        let startup: Arc<dyn Provider> = ModelEcho::on("startup-model");
        // Learning in that chat takes the Settings override; a one-shot in the same chat does not.
        let learning = with_aux_session("sp-one-shot", async { aux_provider(&startup, "startup-model").unwrap().model() }).await;
        let one_shot = with_aux_session("sp-one-shot", with_aux_consumer(crate::factr_config::AuxConsumer::OneShot, async {
            aux_provider(&startup, "startup-model").unwrap().model()
        }))
        .await;
        match before {
            Some(v) => crate::env::set_var("FACTR_CONFIG_HOME", v),
            None => crate::env::remove_var("FACTR_CONFIG_HOME"),
        }
        assert_eq!(one_shot, "chat-model", "the one-shot runs on the chat's own model");
        assert_ne!(learning, "chat-model", "control: the learning consumer did take the override");
        // Without a chat id a one-shot still has no override.
        let bare = with_aux_consumer(crate::factr_config::AuxConsumer::OneShot, async { aux_provider(&startup, "startup-model").unwrap().model() }).await;
        assert_eq!(bare, "startup-model");
    }

    #[tokio::test]
    async fn an_explicit_auxiliary_model_wins_over_the_session_model() {
        let session: Arc<dyn Provider> = ModelEcho::on("chat-model");
        register_session_provider("sp-aux", &session);
        let base = session_provider_fork("sp-aux").unwrap();
        let aux = crate::factr_config::switched(base, Some("aux-model"), "test");
        assert_eq!(aux.model(), "aux-model");
        assert_eq!(session.model(), "chat-model");
    }
}
