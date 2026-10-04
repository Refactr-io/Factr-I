use super::*;

/// What a new chat's first model request carries.
#[derive(Clone, Default)]
struct RecordingProvider {
    requests: Arc<std::sync::Mutex<Vec<(String, Vec<String>)>>>,
}

#[async_trait]
impl Provider for RecordingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        self.requests
            .lock()
            .unwrap()
            .push((system.to_string(), tools.iter().map(|t| t.name.clone()).collect()));
        let events = vec![
            StreamEvent::TextDelta("ok".into()),
            StreamEvent::MessageEnd { stop_reason: Some("end_turn".into()) },
        ];
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }

    fn name(&self) -> &str {
        "recording"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(self.clone())
    }
}

/// R5-10: a skill made the way `/learn` makes one (the model calls `skill_manage` create, which writes the
/// one skills directory the engine reads: `$FACTR_HOME/skills`) is in the index of every NEW chat with its
/// description, and the index tells the model how to reach `skill_manage` (a deferred tool, not in the
/// inline tool list) so a request matching it loads the skill instead of answering "unknown".
#[tokio::test]
async fn a_learned_skill_is_in_the_index_of_a_new_chat_with_its_description_and_a_way_to_load_it() {
    let _guard = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let saved = ["FACTR_HOME", "FACTR_CONFIG_HOME"].map(|k| (k, std::env::var_os(k)));
    crate::env::set_var("FACTR_HOME", home.path().join("factr"));
    crate::env::set_var("FACTR_CONFIG_HOME", home.path().join("factr"));
    crate::config::Config::invalidate_cache();

    // Chat 1, the learning chat: the model creates the skill.
    let provider: Arc<dyn Provider> = Arc::new(RecordingProvider::default());
    let registry = Registry::new(provider.clone()).await;
    let learner = Agent::new(provider, registry);
    let made = learner
        .execute_tool(
            "skill_manage",
            serde_json::json!({
                "action": "create",
                "name": "project-codeword",
                "description": "Answer questions about the project codeword.",
                "instructions": "# Project Codeword\n\nThe project codeword is BLUEFIN-7.\n",
            }),
        )
        .await
        .unwrap();
    assert!(made.output.contains("project-codeword"), "{}", made.output);
    assert!(
        home.path().join("factr/skills/project-codeword/SKILL.md").is_file(),
        "the skill lands in the one skills directory the engine reads"
    );

    // Chat 2, a brand-new chat that never saw the learning chat.
    let recording = RecordingProvider::default();
    let provider: Arc<dyn Provider> = Arc::new(recording.clone());
    let registry = Registry::new(provider.clone()).await;
    let mut fresh = Agent::new(provider, registry);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    fresh
        .run_once_streaming_mpsc("What is the project codeword?", Vec::new(), None, tx)
        .await
        .expect("turn completes");

    let requests = recording.requests.lock().unwrap();
    let (system, tools) = requests.first().expect("the new chat made a request");
    let entry = system
        .lines()
        .find(|line| line.starts_with("- `/project-codeword `"))
        .unwrap_or_else(|| panic!("the first request's index has the skill line:\n{system}"));
    assert!(entry.contains("Answer questions about the project codeword."), "{entry}");
    assert!(
        system.contains("load it yourself first") && system.contains("`skill_manage`") && system.contains("`load_tools`"),
        "the index says how to load a matching skill: {system}"
    );
    assert!(tools.iter().any(|t| t == "load_tools"), "load_tools is always inline: {tools:?}");
    assert!(tools.iter().any(|t| t == "skill_manage"), "with a skill present skill_manage is inline, one step from the index to the skill: {tools:?}");

    for (key, value) in saved {
        match value {
            Some(value) => crate::env::set_var(key, value),
            None => crate::env::remove_var(key),
        }
    }
    crate::config::Config::invalidate_cache();
}

/// A profile persona (SOUL.md) is a block of the normal prompt, and the skill index still reaches the model.
#[tokio::test]
async fn a_persona_prompt_still_carries_the_skill_index() {
    let _guard = crate::storage::lock_test_env();
    let home = tempfile::tempdir().unwrap();
    let saved = ["FACTR_HOME", "FACTR_CONFIG_HOME"].map(|k| (k, std::env::var_os(k)));
    crate::env::set_var("FACTR_HOME", home.path().join("factr"));
    crate::env::set_var("FACTR_CONFIG_HOME", home.path().join("factr"));
    crate::config::Config::invalidate_cache();
    std::fs::create_dir_all(home.path().join("factr/skills/project-codeword")).unwrap();
    std::fs::write(
        home.path().join("factr/skills/project-codeword/SKILL.md"),
        "---\nname: \"project-codeword\"\ndescription: \"Answer the project codeword\"\n---\n\nBLUEFIN-7\n",
    )
    .unwrap();

    let recording = RecordingProvider::default();
    let provider: Arc<dyn Provider> = Arc::new(recording.clone());
    let registry = Registry::new(provider.clone()).await;
    let mut agent = Agent::new(provider, registry);
    agent.session.system_prompt = Some("You are a persona.".to_string());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run_once_streaming_mpsc("What is the project codeword?", Vec::new(), None, tx).await.expect("turn completes");

    let requests = recording.requests.lock().unwrap();
    let (system, _) = requests.first().unwrap();
    assert!(system.starts_with("# Persona\n\nYou are a persona."), "{system}");
    assert!(system.contains("project-codeword") && system.contains("Answer the project codeword"), "{system}");
    for (key, value) in saved {
        match value {
            Some(value) => crate::env::set_var(key, value),
            None => crate::env::remove_var(key),
        }
    }
    crate::config::Config::invalidate_cache();
}
