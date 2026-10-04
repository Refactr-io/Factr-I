use super::Agent;
use crate::logging;
use crate::message::{Message, ToolDefinition};

impl Agent {
    /// Explicitly prepare/freeze the same tool surface used by provider turns.
    /// Unlike `debug_context`, this may update the tool cache. It never calls a provider.
    pub async fn prepare_debug_context(&mut self) -> serde_json::Value {
        let prepared_tools = self.tool_definitions().await;
        let mut context = self.debug_context().await;
        context["prepared_tools"] = serde_json::json!(prepared_tools);
        context
    }

    /// Inspect the next request's static context without inference, prewarming,
    /// or locking a new tool snapshot. Pending memory is deliberately not consumed.
    pub async fn debug_context(&self) -> serde_json::Value {
        let prompt = self.build_system_prompt_split(None);
        let current_tools = self.tool_definitions_for_debug().await;
        let effective_tools = self.locked_tools.as_ref().unwrap_or(&current_tools);
        let locked_tool_names = self.locked_tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
        });
        serde_json::json!({
            "session_id": self.session.id,
            "working_dir": self.session.working_dir,
            "mode": if self.session.is_canary { "cli" } else { "regular" },
            "is_canary": self.session.is_canary,
            "system_prompt": {
                "static": prompt.static_part,
                "dynamic": prompt.dynamic_part,
                "pending_memory_included": false,
            },
            "tools_locked": self.locked_tools.is_some(),
            "locked_tool_names": locked_tool_names,
            "effective_tools": effective_tools,
            "current_tools": current_tools,
        })
    }

    pub(super) fn log_prompt_prefix_accounting(
        &self,
        split: &crate::prompt::SplitSystemPrompt,
        tools: &[ToolDefinition],
    ) {
        let system_tokens = split.estimated_tokens();
        let tool_tokens = ToolDefinition::aggregate_prompt_token_estimate(tools);
        let prefix_tokens = system_tokens + tool_tokens;
        logging::info(&format!(
            "Prompt prefix estimate: total={} tokens (system={} tools={})",
            prefix_tokens, system_tokens, tool_tokens
        ));
    }

    pub(super) fn build_memory_prompt_nonblocking_shared(
        &self,
        messages: std::sync::Arc<[Message]>,
        _memory_event_tx: Option<crate::memory::MemoryEventSink>,
    ) -> Option<crate::memory::PendingMemory> {
        if !self.memory_enabled {
            return None;
        }

        let session_id = &self.session.id;

        let fresh_user_turn = crate::message::ends_with_fresh_user_turn(&messages);
        if fresh_user_turn {
            forget_stale_addenda(session_id);
        }
        if fresh_user_turn && crate::memory_extract::note_user_turn(session_id) {
            self.extract_memories(crate::memory_extract::Trigger::Periodic);
        }
        if fresh_user_turn {
            crate::memory_agent::recall_local_now(
                session_id,
                &messages,
                self.session.working_dir.as_deref(),
            );
        }
        let pending = if fresh_user_turn {
            crate::memory::take_pending_memory_for_project(
                session_id,
                self.session.working_dir.as_deref(),
            )
        } else {
            None
        };

        pending
    }

    /// Extract new memories from this session's messages that were not yet covered (spawned,
    /// never blocks). Session end, the 12-turn periodic run and compaction all come through here
    /// or through the same `memory_extract::spawn`.
    pub(crate) fn extract_memories(&self, trigger: crate::memory_extract::Trigger) {
        if !self.memory_enabled {
            return;
        }
        let messages = &self.session.messages;
        crate::memory_extract::spawn(
            trigger,
            &self.session.id,
            self.session.working_dir.as_deref(),
            messages.len(),
            |from| messages[from..].iter().map(|m| m.to_message()).collect(),
        );
    }

    /// The turn's system reminder as a tail `<system-reminder>` user message, sent with the request
    /// and never persisted. It changes per turn, so it must sit after every cacheable byte (the
    /// system prompt and the history), like memory and nudges, not inside the system prompt.
    pub(super) fn turn_system_reminder_message(&self) -> Option<Message> {
        let reminder = self.current_turn_system_reminder.as_deref()?.trim();
        (!reminder.is_empty())
            .then(|| Message::user(&format!("<system-reminder>\n{reminder}\n</system-reminder>")))
    }

    /// The skills index, rendered once per session (see `skills_index` on `Agent`): most recently
    /// touched skills first, up to the index token budget.
    fn skills_index(&self) -> &Option<String> {
        self.skills_index.get_or_init(|| {
            let disabled = factr_base::skill::disabled_skill_names();
            // The shipped `learn-*` REPL wrappers are documented in the REPL guidance below and stay
            // loadable with `skill_manage`; listing them would only spend tokens on every request.
            let shipped = factr_learn::shipped_skill_dirs();
            let available_skills: Vec<crate::prompt::SkillInfo> = self
                .current_skills_snapshot()
                .list_recent_first()
                .iter()
                .filter(|skill| !disabled.contains(&skill.name) && !shipped.contains(&skill.name.as_str()))
                .map(|skill| crate::prompt::SkillInfo {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                })
                .collect();
            crate::prompt::build_available_skills_section(&available_skills)
        })
    }

    /// Build split system prompt for better caching
    /// Returns static (cacheable) and dynamic (not cached) parts separately
    pub(super) fn build_system_prompt_split(
        &self,
        memory_prompt: Option<&str>,
    ) -> crate::prompt::SplitSystemPrompt {
        let skills = self.current_skills_snapshot();
        let skill_prompt = self
            .active_skill
            .as_ref()
            .and_then(|name| skills.get(name).map(|skill| skill.get_prompt().to_string()));

        let working_dir = self
            .session
            .working_dir
            .as_ref()
            .map(std::path::PathBuf::from);

        let (mut split, _context_info) = crate::prompt::build_system_prompt_split_with_snapshot(
            skill_prompt.as_deref(),
            self.skills_index().clone(),
            self.session.is_canary,
            memory_prompt,
            working_dir.as_deref(),
            self.prompt_snapshot.clone(),
        );

        self.append_continual_harness_addenda(&mut split);
        self.append_repl_guidance(&mut split);
        self.append_tool_use_enforcement(&mut split);
        crate::prompt::append_swarm_effort_directive(
            &mut split,
            self.provider.reasoning_effort().as_deref(),
        );
        self.prepend_persona(&mut split);

        split
    }

    /// The session's persona (the profile's SOUL.md, or a caller's `system_prompt`) is a block at the
    /// top of the normal prompt, never a replacement for it: the harness addenda, AGENTS.md, the
    /// memory prompt, tool guidance and the skill index all still apply.
    fn prepend_persona(&self, split: &mut crate::prompt::SplitSystemPrompt) {
        let Some(persona) = self.session.system_prompt.as_deref().map(str::trim).filter(|p| !p.is_empty()) else {
            return;
        };
        split.static_part = format!(
            "# Persona\n\n{persona}\n\nThe persona above is who you are and how you speak; \
             the instructions below are how you work, and they never rename you.\n\n{}",
            split.static_part
        );
    }

    /// factr-learn's Continual Harness `prompt` notes (memories of category `prompt`), rendered into the
    /// *static* (cached) part so provider prompt caching still applies: a
    /// running session's cache stays valid because this only changes when a
    /// brand-new session builds its first prompt, matching M9's "applied to
    /// new sessions" contract for `/refine`. Factr engine only (the same
    /// engine gate as the REPL; no Python needed), and best-effort:
    /// any storage error here must never break prompt building.
    fn append_continual_harness_addenda(&self, split: &mut crate::prompt::SplitSystemPrompt) {
        if std::env::var_os("FACTR_REPL_WORKER").is_none() {
            return;
        }
        // Snapshot per session: a note learned elsewhere must not change a running session's static
        // prefix mid-turn (it would break its prompt cache). It is rebuilt at the next user turn
        // once a learned row changed ([`forget_stale_addenda`]); unchanged text keeps the cache.
        let cached = addenda_snapshots()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.session.id)
            .map(|(_, addenda)| addenda.clone());
        if let Some(addenda) = cached {
            push_addenda(split, &addenda);
            return;
        }
        let Ok(home) = factr_base::storage::factr_dir() else {
            return;
        };
        let Ok(store) = factr_learn::entries::EntryStore::open_cached(&home) else {
            return;
        };
        if let Some(dir) = self.session.working_dir.as_deref() {
            let _ = store.set_session_dir(&self.session.id, dir);
        }
        // Read before rendering: a row written meanwhile makes this snapshot stale, never fresh.
        let generation = factr_base::memory::learned::generation();
        let Ok((addenda, note_ids)) = store.render_prompt_with_ids(&self.session.id) else {
            return;
        };
        // Behaviour rules are always here, so recall must never show them again.
        crate::memory::pin_known(&self.session.id, &note_ids);
        let addenda = addenda.trim().to_string();
        addenda_snapshots()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.session.id.clone(), (generation, addenda.clone()));
        push_addenda(split, &addenda);
    }

    /// Factr `TOOL_USE_ENFORCEMENT` for the model families it lists (never
    /// Claude). Static part, so the prompt cache prefix stays stable.
    fn append_tool_use_enforcement(&self, split: &mut crate::prompt::SplitSystemPrompt) {
        if !needs_tool_use_enforcement(&self.provider.model()) {
            return;
        }
        split.static_part.push_str(
            "\n\nWhen you say you will act, make the tool call in the same response. Every reply either calls a tool or gives the final result.\n",
        );
    }

    /// The REPL paragraph, only where the REPL can run (it is the sole owner of this guidance).
    /// Static per process, so the prompt cache prefix stays stable.
    fn append_repl_guidance(&self, split: &mut crate::prompt::SplitSystemPrompt) {
        if !crate::tool::repl_available() {
            return;
        }
        if !split.static_part.is_empty() {
            split.static_part.push_str("\n\n");
        }
        split.static_part.push_str(&repl_guidance(factr_learn::host::sandbox_available()));
    }

    /// Non-blocking memory prompt - takes pending result and spawns check for next turn
    #[cfg(test)]
    pub(super) fn build_memory_prompt_nonblocking(
        &self,
        messages: &[Message],
        _memory_event_tx: Option<crate::memory::MemoryEventSink>,
    ) -> Option<crate::memory::PendingMemory> {
        self.build_memory_prompt_nonblocking_shared(messages.to_vec().into(), _memory_event_tx)
    }
}

/// Without the macOS sandbox the worker runs only on an explicit `FACTR_REPL_PYTHON`, with bash's
/// network and file access, so the network line must say so.
fn repl_guidance(sandboxed: bool) -> String {
    let network = if sandboxed { "no network" } else { "network and files as in bash (no sandbox here)" };
    format!(
        "## Recursive REPL\n\nLoad the `repl` tool with `load_tools` to keep large inputs in persistent Python variables: `await load(path, start, length)` reads a slice, `await llm_query(prompt)` asks a sub-model, `await llm_query_batch(prompts)` runs up to 64 at once. Engine features: `await refine('run', instructions)`, `await goal(op, objective)`, `await heartbeat(op, ...)`, `await spawn_subagent(prompt, name)`, `await agent_message(action, message, target)`. Every helper is async: always `await` it. Print only what you need. Limits: the standard library plus the packages of the same Python that `python` runs in bash, {network}, 20 s of compute per cell (a longer cell is interrupted and keeps its variables), a memory cap (a worker over it is restarted), variables persist between calls.\n"
    )
}

fn needs_tool_use_enforcement(model: &str) -> bool {
    const FAMILIES: [&str; 9] = [
        "gpt", "codex", "gemini", "gemma", "grok", "glm", "qwen", "deepseek", "muse",
    ];
    let model = model.to_ascii_lowercase();
    FAMILIES.iter().any(|f| model.contains(f))
}

/// Per session: the learned-rows generation the notes were rendered at, and the rendered text.
type AddendaSnapshots = std::sync::Mutex<std::collections::HashMap<String, (u64, String)>>;

fn addenda_snapshots() -> &'static AddendaSnapshots {
    static M: std::sync::OnceLock<AddendaSnapshots> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Drop `session_id`'s snapshot when a learned row changed since it was built, so the next prompt
/// build renders the notes again. Called only at a fresh user turn: never mid-turn.
fn forget_stale_addenda(session_id: &str) {
    let mut snapshots = addenda_snapshots().lock().unwrap_or_else(|e| e.into_inner());
    if snapshots.get(session_id).is_some_and(|(at, _)| *at != factr_base::memory::learned::generation()) {
        snapshots.remove(session_id);
    }
}

/// A closed session's snapshot and pinned ids: nothing per-session outlives it.
pub(crate) fn forget_addenda(session_id: &str) {
    addenda_snapshots().lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
    crate::memory::unpin_known(session_id);
}

fn push_addenda(split: &mut crate::prompt::SplitSystemPrompt, addenda: &str) {
    if addenda.is_empty() {
        return;
    }
    if !split.static_part.is_empty() {
        split.static_part.push_str("\n\n");
    }
    split.static_part.push_str("# Continual Harness\n\n");
    split.static_part.push_str(addenda);
}

#[cfg(test)]
mod addenda_snapshot_tests {
    use super::*;

    #[test]
    fn tool_use_enforcement_skips_claude() {
        assert!(needs_tool_use_enforcement("gpt-5.1-codex"));
        assert!(needs_tool_use_enforcement("Qwen3-Coder"));
        assert!(!needs_tool_use_enforcement("claude-sonnet-4-5"));
    }

    #[test]
    fn repl_guidance_claims_no_network_only_under_the_sandbox() {
        let sandboxed = repl_guidance(true);
        assert!(sandboxed.contains("no network") && sandboxed.contains("load_tools") && sandboxed.contains("await refine("));
        let open = repl_guidance(false);
        assert!(!open.contains("no network") && open.contains("no sandbox here"));
    }

    #[test]
    fn snapshot_is_reused_and_not_refreshed() {
        addenda_snapshots().lock().unwrap().insert("s-snap".into(), (0, "rule A".into()));
        let mut split = crate::prompt::SplitSystemPrompt::default();
        push_addenda(&mut split, "rule A");
        assert!(split.static_part.ends_with("rule A"));
        // a later insert for another session leaves this one untouched
        addenda_snapshots().lock().unwrap().insert("other".into(), (0, "rule B".into()));
        assert_eq!(addenda_snapshots().lock().unwrap()["s-snap"].1, "rule A");
    }

    #[test]
    fn a_changed_learned_row_drops_the_snapshot_at_the_next_user_turn_and_a_closed_session_loses_it() {
        let current = factr_base::memory::learned::generation();
        addenda_snapshots().lock().unwrap().insert("s-gen".into(), (current, "rule A".into()));
        addenda_snapshots().lock().unwrap().insert("s-gen-stale".into(), (current.wrapping_sub(1), "rule A".into()));
        crate::memory::pin_known("s-gen", &["note-1".to_string()]);
        forget_stale_addenda("s-gen");
        forget_stale_addenda("s-gen-stale");
        let snapshots = addenda_snapshots().lock().unwrap();
        assert!(snapshots.contains_key("s-gen"), "an unchanged store keeps the snapshot, and the prompt cache");
        assert!(!snapshots.contains_key("s-gen-stale"), "a stale one is rebuilt");
        drop(snapshots);
        forget_addenda("s-gen");
        assert!(!addenda_snapshots().lock().unwrap().contains_key("s-gen"));
        assert!(!crate::memory::is_memory_injected("s-gen", "note-1"), "and its pins");
    }
}
