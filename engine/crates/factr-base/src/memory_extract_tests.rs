use super::*;
use crate::memory::TrustLevel;
use crate::message::Message;
use std::sync::{Arc, Mutex as StdMutex};

fn with_temp_home<T>(f: impl FnOnce() -> T) -> T {
    let _guard = crate::storage::lock_test_env();
    let old = std::env::var("FACTR_HOME").ok();
    let dir = tempfile::TempDir::new().expect("temp home");
    crate::env::set_var("FACTR_HOME", dir.path());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    match old {
        Some(value) => crate::env::set_var("FACTR_HOME", value),
        None => crate::env::remove_var("FACTR_HOME"),
    }
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn chat(pairs: usize) -> Vec<Message> {
    (0..pairs)
        .flat_map(|i| {
            [
                Message::user(&format!("Question {i}: always write the release tooling in Nim, I prefer it. The release tooling lives in the tools directory.")),
                Message::assistant_text(&format!("Answer {i}: noted, I will keep the release tooling in Nim and tell you where it lives.")),
            ]
        })
        .collect()
}

fn plan_run(
    manager: &MemoryManager,
    trigger: Trigger,
    session: &str,
    messages: &[Message],
    enabled: bool,
) -> Plan {
    plan(manager, trigger, session, messages.len(), enabled, |from| messages[from..].to_vec())
}

/// A fake provider: answers with `reply` and records the system prompts it was given.
fn fake(reply: &str, seen: Arc<StdMutex<Vec<String>>>) -> impl Fn(String, String) -> BoxFuture<'static, Result<Completion>> + Send + Sync {
    let reply = reply.to_string();
    move |system, _prompt| {
        seen.lock().unwrap().push(system);
        let text = reply.clone();
        Box::pin(async move { Ok(Completion { text, input_tokens: 10, output_tokens: 5, model: "fake".into() }) })
    }
}

fn run(manager: &MemoryManager, job: Job, complete: Complete<'_>) -> Vec<String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(run_job(manager, job, complete))
}

fn job_of(plan: Plan) -> Job {
    match plan {
        Plan::Run(job) => job,
        Plan::Skip(reason) => panic!("expected a run, skipped: {reason}"),
    }
}

fn skipped(plan: Plan) -> &'static str {
    match plan {
        Plan::Skip(reason) => reason,
        Plan::Run(_) => panic!("expected a skip"),
    }
}

#[test]
fn parser_reads_four_fields_and_marks_old_formats_unverifiable() {
    let parsed = parse_extracted(
        "Here you go:\nPreference|Prefers Nim for scripts|3|always Nim | really\nfact| Uses SQLite |[#2]|uses sqlite\nno pipes here\nfact|only two\nentity||1|q\ncorrection|a|b|c\nfact|Old format line|high",
    );
    assert_eq!(parsed.len(), 4);
    assert_eq!(parsed[0], Extracted { category: "preference".into(), content: "Prefers Nim for scripts".into(), msg_index: Some(3), quote: "always Nim | really".into() });
    assert_eq!((parsed[1].content.as_str(), parsed[1].msg_index), ("Uses SQLite", Some(2)));
    assert_eq!(parsed[2].msg_index, None, "non-numeric index");
    assert_eq!(parsed[3].msg_index, None, "3-field format has nothing to verify");
    assert!(parse_extracted("").is_empty());
}

#[test]
fn transcript_strips_reminders_and_takes_oldest_first_within_cap() {
    let mut messages = vec![Message::user("<system-reminder>secret boilerplate</system-reminder>Hello there")];
    messages.push(Message::assistant_text("old reply that should fall off the cap"));
    messages.push(Message::user("newest question"));
    let (all, consumed) = build_window(&messages, usize::MAX, 10_000);
    assert!(all.contains("Hello there") && !all.contains("secret boilerplate"));
    assert_eq!(consumed, 3);
    // Oldest first: what does not fit stays unconsumed for the next window.
    let (capped, consumed) = build_window(&messages, usize::MAX, 60);
    assert!(capped.contains("Hello there") && !capped.contains("newest question"), "{capped}");
    assert_eq!(consumed, 1, "only the messages actually included are consumed");
    let (one, consumed) = build_window(&messages, 1, 10_000);
    assert!(one.contains("Hello there") && consumed == 1);
    assert_eq!(strip_system_reminders("a<system-reminder>x</system-reminder>b<system-reminder>open"), "ab");
}

#[test]
fn floors_gate_short_conversations() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let three = chat(2)[..3].to_vec();
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "floor-a", &three, true)), "under_floor");
        let tiny: Vec<Message> = (0..6).map(|_| Message::user("hi")).collect();
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "floor-b", &tiny, true)), "under_floor");
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "floor-c", &chat(3), false)), "sidecar_off");
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "floor-d", &[], true)), "no_new_messages");
        assert!(matches!(plan_run(&manager, Trigger::SessionEnd, "floor-e", &chat(3), true), Plan::Run(_)));
    });
}

#[test]
fn a_stored_user_correction_or_preference_is_a_learning_signal_and_a_fact_is_not() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let messages = chat(3);
        let fact = "fact|Release tooling lives in the tools directory|0|The release tooling lives in the tools directory";
        let job = job_of(plan_run(&manager, Trigger::Compaction, "sig-1", &messages, true));
        run(&manager, job, &fake(fact, seen.clone()));
        assert_eq!(crate::learn_signal::take("sig-1"), 0, "a fact is not a correction");

        let pref = "preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim";
        let job = job_of(plan_run(&manager, Trigger::Compaction, "sig-2", &messages, true));
        run(&manager, job, &fake(pref, seen.clone()));
        assert_eq!(crate::learn_signal::take("sig-2"), 1);
        assert_eq!(crate::learn_signal::take("sig-2"), 0, "taken once");

        // The closing session leaves nothing behind.
        crate::learn_signal::note("sig-3");
        forget_session("sig-3");
        assert_eq!(crate::learn_signal::take("sig-3"), 0);
    });
}

#[test]
fn a_note_the_session_learned_for_itself_is_in_the_already_known_list() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let note = crate::memory::MemoryEntry::new(crate::memory::MemoryCategory::Custom("prompt".into()), "Write the release tooling in Nim");
        crate::memory::learned::put(&manager.db_path().unwrap(), "session:rel-1", note).unwrap();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let messages = chat(3);
        for session in ["rel-1", "rel-2"] {
            let job = job_of(plan_run(&manager, Trigger::Compaction, session, &messages, true));
            run(&manager, job, &fake("", seen.clone()));
        }
        let seen = seen.lock().unwrap();
        assert!(seen[0].contains("- Write the release tooling in Nim"), "its own session's note is known: {}", seen[0]);
        assert!(!seen[1].contains("Write the release tooling in Nim"), "another session's is not");
    });
}

#[test]
fn extraction_stores_dedupes_and_marks_the_marker() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let messages = chat(3);
        let job = job_of(plan_run(&manager, Trigger::Compaction, "ex-1", &messages, true));
        let ids = run(&manager, job, &fake("preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim\nfact|Release tooling lives in the tools directory|0|The release tooling lives in the tools directory\nfact|The assistant will keep tooling in Nim|1|I will keep the release tooling in Nim\nfact|Tooling is also in Go|high", seen.clone()));
        assert_eq!(ids.len(), 2, "assistant-only and old-format lines are rejected");
        assert!(manager.list_all().unwrap().iter().all(|e| e.trust == TrustLevel::High), "user-stated quotes are high trust, set by code");
        assert_eq!(manager.list_all().unwrap().len(), 2);
        assert!(!seen.lock().unwrap()[0].contains("Already known (do NOT"), "empty store, no known list");

        // A later run of the same session re-extracts a paraphrase: it merges, no second row.
        forget_session("ex-1");
        let mut more = messages.clone();
        more.extend(chat(3));
        let job = job_of(plan_run(&manager, Trigger::Compaction, "ex-1", &more, true));
        assert_eq!(job.upto, 12);
        let ids2 = run(&manager, job, &fake("preference|The user prefers Nim for release scripting|0|always write the release tooling in Nim", seen.clone()));
        assert!(ids2.is_empty(), "a paraphrase of a known memory is rejected before it is written");
        let active: Vec<_> = manager.list_all().unwrap().into_iter().filter(|e| e.active).collect();
        assert_eq!(active.len(), 2, "paraphrase did not create a second active row");
        let prompt = seen.lock().unwrap()[1].clone();
        assert!(prompt.contains("Already known (do NOT") && prompt.contains("Prefers Nim"), "{prompt}");
        assert!(active.iter().all(|e| e.source.as_deref() == Some("ex-1")));

        // The marker moved: the same history yields nothing new, a longer one only the tail.
        forget_session("ex-1");
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "ex-1", &more, true)), "no_new_messages");
        let mut longer = more.clone();
        longer.extend(chat(3));
        let job = job_of(plan_run(&manager, Trigger::SessionEnd, "ex-1", &longer, true));
        assert_eq!(job.upto, 18);
        assert!(!job.transcript.is_empty());
    });
}

#[test]
fn failure_leaves_the_marker_so_the_window_is_retried() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let messages = chat(3);
        let job = job_of(plan_run(&manager, Trigger::SessionEnd, "fail-1", &messages, true));
        let failing = |_s: String, _p: String| -> BoxFuture<'static, Result<Completion>> { Box::pin(async { Err(anyhow!("provider down")) }) };
        assert!(run(&manager, job, &failing).is_empty());
        forget_session("fail-1");
        assert!(matches!(plan_run(&manager, Trigger::SessionEnd, "fail-1", &messages, true), Plan::Run(_)));
    });
}

#[test]
fn cooldown_spaces_periodic_runs_but_never_blocks_session_end_or_compaction() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let messages = chat(3);
        assert!(matches!(plan_run(&manager, Trigger::Periodic, "cool-1", &messages, true), Plan::Run(_)));
        assert_eq!(skipped(plan_run(&manager, Trigger::Periodic, "cool-1", &messages, true)), "in_flight");
        release("cool-1");
        assert_eq!(skipped(plan_run(&manager, Trigger::Periodic, "cool-1", &messages, true)), "cooldown");
        assert!(matches!(plan_run(&manager, Trigger::SessionEnd, "cool-1", &messages, true), Plan::Run(_)), "session end is not cooled");
        assert!(matches!(plan_run(&manager, Trigger::Compaction, "cool-1", &messages, true), Plan::Run(_)));
        forget_session("cool-1");
        assert!(matches!(plan_run(&manager, Trigger::Periodic, "cool-1", &messages, true), Plan::Run(_)));
    });
}

#[test]
fn every_twelfth_user_turn_is_periodic_and_state_is_pruned() {
    let hits: Vec<usize> = (1..=24).filter(|_| note_user_turn("turns-1")).collect();
    assert_eq!(hits.len(), 2);
    forget_session("turns-1");
    assert!(!note_user_turn("turns-1"), "counter restarted after close");
    forget_session("turns-1");
    assert!(SESSIONS.lock().unwrap().get("turns-1").is_none());
}

#[test]
fn periodic_run_takes_at_most_forty_messages_and_capped_chars() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let messages = chat(60);
        let job = job_of(plan_run(&manager, Trigger::Periodic, "cap-1", &messages, true));
        assert_eq!(job.transcript.matches("] User:").count(), 20);
        assert!(job.transcript.contains("Question 0:") && !job.transcript.contains("Question 59:"), "oldest window first");
        assert_eq!(job.upto, 40, "only the 40 messages included are marked");
        let big = vec![Message::user(&"x".repeat(30_000)); 5];
        let job = job_of(plan_run(&manager, Trigger::SessionEnd, "cap-2", &big, true));
        assert!(job.transcript.chars().count() <= MAX_TRANSCRIPT_CHARS);
    });
}

#[test]
fn extraction_without_working_dir_writes_global() {
    with_temp_home(|| {
        let manager = MemoryManager::new();
        assert!(manager.remember_extracted(MemoryEntry::new(MemoryCategory::Fact, "A global fact about builds")).is_ok());
        assert_eq!(manager.list_all().unwrap().len(), 1);
    });
}

#[test]
fn detailed_recall_counts_candidates_and_suppressed() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        for text in ["Release tooling is written in Nim", "Release notes go in the changelog file", "Unrelated fact about gardening"] {
            manager.remember_project(MemoryEntry::new(MemoryCategory::Fact, text)).unwrap();
        }
        let recall = manager
            .recall_local_detailed(Some("recall-1"), "where does the release tooling live", 5, crate::memory::MemoryScope::All)
            .unwrap();
        assert!(recall.candidates >= recall.entries.len());
        assert_eq!(recall.suppressed, recall.candidates - recall.entries.len());
        assert!(recall.terms.contains(&"release".to_string()));
        let first = recall.entries[0].id.clone();
        crate::memory::mark_memories_known("recall-1", &[first], "test");
        let again = manager.recall_local_detailed(Some("recall-1"), "where does the release tooling live", 5, crate::memory::MemoryScope::All).unwrap();
        assert!(again.suppressed >= 1);
        crate::memory::clear_injected_memories("recall-1");
    });
}

#[test]
fn oversized_backlog_is_consumed_across_windows_without_skipping() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let big: Vec<Message> = (0..6).map(|i| Message::user(&format!("m{i} {}", "x".repeat(10_000)))).collect();
        let job = job_of(plan_run(&manager, Trigger::Compaction, "win-1", &big, true));
        assert_eq!(job.upto, 2, "24k cap fits two 10k messages");
        assert!(job.transcript.contains("m0 ") && job.transcript.contains("m1 ") && !job.transcript.contains("m2 "));
        run(&manager, job, &fake("", seen.clone()));
        assert_eq!(seen.lock().unwrap().len(), 3, "compaction drains the whole range window by window");
        assert_eq!(read_marker(&manager, "win-1"), 6);
    });
}

#[test]
fn periodic_leaves_the_rest_for_the_next_run() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let messages = chat(60);
        let job = job_of(plan_run(&manager, Trigger::Periodic, "per-1", &messages, true));
        run(&manager, job, &fake("", seen.clone()));
        assert_eq!(read_marker(&manager, "per-1"), 40);
        forget_session("per-1");
        let job = job_of(plan_run(&manager, Trigger::Periodic, "per-1", &messages, true));
        assert_eq!(job.from, 40);
        assert!(job.transcript.contains("Question 20:"));
    });
}

#[test]
fn session_end_after_periodic_does_not_repeat_or_drop() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let messages = chat(60);
        let periodic = job_of(plan_run(&manager, Trigger::Periodic, "end-1", &messages, true));
        // SessionEnd is planned while the periodic job is still in flight, from the same marker.
        let end = job_of(plan_run(&manager, Trigger::SessionEnd, "end-1", &messages, true));
        assert_eq!(skipped(plan_run(&manager, Trigger::Periodic, "end-1", &messages, true)), "in_flight");
        run(&manager, periodic, &fake("", seen.clone()));
        run(&manager, end, &fake("", seen.clone()));
        assert_eq!(seen.lock().unwrap().len(), 2, "periodic window, then only the tail: no repeated window");
        assert_eq!(read_marker(&manager, "end-1"), 120);
    });
}

#[test]
fn a_session_that_pre_dates_extraction_is_stamped_and_only_new_messages_are_extracted() {
    with_temp_home(|| {
        let manager = MemoryManager::new();
        let old = chat(40);
        adopt_session("legacy-1", old.len());
        assert_eq!(extracted_through(&manager, "legacy-1", old.len()), old.len());
        assert!(matches!(plan_run(&manager, Trigger::SessionEnd, "legacy-1", &old, true), Plan::Skip("no_new_messages")));
        let mut grown = old.clone();
        grown.extend(chat(3));
        let job = job_of(plan_run(&manager, Trigger::SessionEnd, "legacy-1", &grown, true));
        assert_eq!(job.from, old.len());
        // Adopting again, or after real activity, never moves a marker.
        adopt_session("legacy-1", grown.len());
        assert_eq!(extracted_through(&manager, "legacy-1", grown.len()), old.len());
        forget_session("legacy-1");
    });
}

#[test]
fn undo_moves_the_marker_down_instead_of_resetting_it() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        manager.meta_set(&through_key("undo-1"), "20").unwrap();
        assert_eq!(extracted_through(&manager, "undo-1", 10), 10);
        assert_eq!(read_marker(&manager, "undo-1"), 10, "marker follows the shorter history");
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "undo-1", &chat(5), true)), "no_new_messages", "no re-extraction of the kept turns");
        let job = job_of(plan_run(&manager, Trigger::SessionEnd, "undo-1", &chat(8), true));
        assert_eq!(job.from, 10, "only turns after the undo point are read");
    });
}

#[test]
fn rewind_deactivates_memories_past_the_cut_and_redo_restores_them_and_the_marker() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let job = job_of(plan_run(&manager, Trigger::Compaction, "rw-1", &chat(3), true));
        let ids = run(&manager, job, &fake("preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim", seen));
        assert_eq!(ids.len(), 1);
        assert_eq!(read_marker(&manager, "rw-1"), 6);

        // A cut that keeps the whole extracted range changes nothing.
        let none = rewind_memories_in(&manager, "rw-1", 6);
        assert!(none.ids.is_empty() && none.marker == 6);

        let undone = rewind_memories_in(&manager, "rw-1", 0); // cuts the source message
        assert_eq!(undone.ids.len(), 1);
        assert!(manager.list_all().unwrap().iter().all(|e| !e.active), "undone turn's memory is deactivated");
        assert!(read_ledger(&manager, "rw-1").is_empty());

        manager.meta_set(&through_key("rw-1"), "0").unwrap(); // what the rewind's marker clamp does
        restore_memories_in(&manager, "rw-1", &undone);
        assert!(manager.list_all().unwrap().iter().all(|e| e.active), "redo reactivates it");
        assert_eq!(read_marker(&manager, "rw-1"), 6, "redo restores the marker: no re-extraction");
        assert_eq!(read_ledger(&manager, "rw-1").len(), 1, "and a later rewind can deactivate it again");
    });
}

#[test]
fn rejection_counts_are_recorded_by_reason() {
    let texts: Vec<_> = [Message::user("Please always use tabs for indentation in this repo.")].iter().map(quality::msg_text).collect();
    let lines = "preference|Always use tabs for indentation|0|always use tabs for indentation\nfact|Made up claim about the build|0|never said this at all\nfact|Another made up claim here|0|also never said this\nfact|Old format|high";
    let (accepted, rejected) = accept(parse_extracted(lines), &texts, &[], &[]);
    assert_eq!(accepted.len(), 1);
    assert_eq!(rejected.get("quote_not_found"), Some(&2));
    assert_eq!(rejected.get("unverifiable"), Some(&1));
}

#[test]
fn at_most_five_memories_per_window() {
    let texts: Vec<_> = [Message::user("Please always use tabs for indentation in this repo.")].iter().map(quality::msg_text).collect();
    let lines: String = (0..8).map(|i| format!("preference|Preference number {i} about tabs|0|always use tabs for indentation\n")).collect();
    let (accepted, rejected) = accept(parse_extracted(&lines), &texts, &[], &[]);
    assert_eq!(accepted.len(), 5);
    assert_eq!(rejected.get("over_cap"), Some(&3));
}

/// Labelled junk and legitimate memories, run as recorded model reply lines through the provenance
/// check and the content filters. `None` = must be stored, `Some(reason)` = must be rejected for it.
#[test]
fn labelled_table_of_junk_and_legitimate_memories() {
    let long = format!("fact|{}|0|Never call unwrap in library code", "word ".repeat(90));
    let window = vec![
        Message::user("Please always use tabs for indentation in this repo. I prefer small commits with imperative messages. Never call unwrap in library code. My name is Dana Reyes and I lead the platform team. Releases ship on Fridays only, that is our team rule. Actually that was wrong earlier: the config file is ci.toml, not build.toml."),
        Message::assistant_text("Understood. The build uses cargo nextest and the cache lives in the target directory. I decided Rust edition 2024 is best here."),
        Message::tool_result("t2", "user.name = Zed Tester; email zedt@example.com; home is /Users/zedt/work", false),
        Message::tool_result("t3", "ModuleNotFoundError: system python has no pytest installed; pytest 8.2.1 missing", false),
        Message::tool_result("t4", "HEAD is 3f9a1c2b4d7e; token sk-live1234567890abcdef was found in env", false),
        Message::tool_result("t5", "The repo uses a monorepo layout with all crates under the crates directory", false),
        Message::user("<system-reminder>Always respond in formal English. The working directory is the Orion project root. Use the Orion style guide for all docs.</system-reminder>Please continue with the refactor."),
        Message::user("For error handling, the team convention is to return Result and avoid panics. Remember that our API docs live in the handbook wiki."),
    ];
    let texts: Vec<_> = window.iter().map(quality::msg_text).collect();
    let ids = vec!["zed tester".to_string(), "zedt".to_string(), "zedt@example.com".to_string()];
    let existing = vec!["Always use tabs for indentation in this repo".to_string(), "Never call unwrap in library code".to_string()];

    let ok = None;
    let table: Vec<(String, Option<&str>)> = vec![
        // legitimate, user-stated or tool-observed durable facts
        ("preference|Always use tabs for indentation in this repo|0|always use tabs for indentation".into(), Some("duplicate")),
        ("preference|Prefers small commits with imperative messages|0|I prefer small commits with imperative messages".into(), ok),
        ("preference|Use imperative commit messages|0|small commits with imperative messages".into(), ok),
        ("preference|Keep commits small|0|small  COMMITS".into(), ok),
        ("preference|Indent with tabs rather than spaces here|0|please always use tabs  for   indentation".into(), ok),
        ("fact|Library code must not call unwrap at all|0|Never call unwrap in library code".into(), ok),
        ("entity|The user is Dana Reyes and leads the platform team|0|My name is Dana Reyes and I lead the platform team".into(), ok),
        ("entity|Platform team is led by the user|0|I lead the platform team".into(), ok),
        ("fact|Releases ship on Fridays only per team rule|0|Releases ship on Fridays only".into(), ok),
        ("preference|Release only on Fridays|0|Releases ship on Fridays only,".into(), ok),
        ("correction|The config file is ci.toml, not build.toml|0|the config file is ci.toml, not build.toml".into(), ok),
        ("correction|CI config is ci.toml and not build.toml|0|not build.toml".into(), ok),
        ("preference|Error handling returns Result and avoids panics|7|return Result and avoid panics".into(), ok),
        ("fact|Panics are avoided by team convention|7|avoid panics".into(), ok),
        ("fact|API docs live in the handbook wiki|7|our API docs live in the handbook wiki".into(), ok),
        ("preference|Docs go in the handbook wiki, not the repo|7|API docs live in the handbook wiki".into(), ok),
        ("fact|The repo is a monorepo with crates under the crates directory|5|monorepo layout with all crates under the crates directory".into(), ok),
        ("Preference|Prefers small commits, imperatively worded|#0|I prefer small commits".into(), ok),
        ("fact|Team rule: Friday releases only|0|that is our team rule".into(), ok),
        ("preference|Tabs, never spaces, for indentation|0|always use tabs for indentation in this repo".into(), ok),
        // old format, no provenance
        ("preference|Prefers Nim for scripting|high".into(), Some("unverifiable")),
        ("fact|Uses SQLite for storage|medium".into(), Some("unverifiable")),
        ("entity|Dana Reyes is the user|high".into(), Some("unverifiable")),
        ("fact|Build is cargo based|low".into(), Some("unverifiable")),
        ("fact|Uses tabs for indentation|0|".into(), Some("unverifiable")),
        // the model's own claims
        ("fact|The build uses cargo nextest|1|The build uses cargo nextest".into(), Some("assistant_only")),
        ("fact|Cache lives in the target directory|1|the cache lives in the target directory".into(), Some("assistant_only")),
        ("preference|Rust edition 2024 is best here|1|Rust edition 2024 is best here".into(), Some("assistant_only")),
        ("correction|The agent decided on edition 2024|1|I decided Rust edition 2024 is best".into(), Some("assistant_only")),
        ("fact|Tests run with cargo nextest only|1|cargo nextest and the cache lives".into(), Some("assistant_only")),
        // fabricated or misplaced quotes
        ("fact|The user wants spaces for indentation|0|please always use spaces for indentation".into(), Some("quote_not_found")),
        ("fact|API docs live in the handbook wiki|0|our API docs live in the handbook wiki".into(), Some("quote_not_found")),
        ("fact|The build is hermetic everywhere|99|hermetic everywhere in this repo".into(), Some("bad_index")),
        ("preference|Prefers tabs always in the repo|0|tabs".into(), Some("weak_quote")),
        // identity copied from tool output
        ("entity|The user's name is Zed Tester|2|user.name = Zed Tester".into(), Some("identity")),
        ("entity|Contact email is zedt@example.com|2|email zedt@example.com".into(), Some("identity")),
        ("fact|Developer zedt works on the application|2|user.name = Zed Tester".into(), Some("identity")),
        // paths
        ("fact|The project lives at /Users/zedt/work/app|2|home is /Users/zedt/work".into(), Some("absolute_path")),
        ("fact|Sources are in /home/ci/build/src|5|monorepo layout with all crates".into(), Some("absolute_path")),
        (r"fact|Project is at C:\Projects\app\src|5|monorepo layout with all crates".into(), Some("absolute_path")),
        ("fact|Config is under ~/.config/tool/ for the app|5|monorepo layout with all crates".into(), Some("absolute_path")),
        // ephemeral environment state
        ("fact|System python has no pytest installed|3|system python has no pytest installed".into(), Some("ephemeral")),
        ("fact|pytest is not installed in this environment|3|ModuleNotFoundError: system python".into(), Some("ephemeral")),
        ("fact|pytest 8.2.1 is missing from the box|3|pytest 8.2.1 missing".into(), Some("ephemeral")),
        ("fact|The build is currently broken right now|0|Releases ship on Fridays only".into(), Some("ephemeral")),
        ("fact|Tests are failing today at the moment|0|Never call unwrap in library code".into(), Some("ephemeral")),
        ("fact|Linter version is v3.12.1 for the repo|5|monorepo layout with all crates".into(), Some("ephemeral")),
        ("fact|Command not found for the linter binary|3|ModuleNotFoundError: system python".into(), Some("ephemeral")),
        ("fact|Needed temporarily for this session only|0|I prefer small commits".into(), Some("ephemeral")),
        // secrets and hashes
        ("fact|The API token is sk-live1234567890abcdef|4|token sk-live1234567890abcdef was found".into(), Some("secret")),
        ("fact|Database password is stored in the wiki page|0|Never call unwrap in library code".into(), Some("secret")),
        ("fact|Push with ghp_abcdefghij1234567890 always|4|token sk-live1234567890abcdef was found".into(), Some("secret")),
        ("fact|Deploy key 9f8e7d6c5b4a39281706f5e4d3c2b1a0 is for prod|5|monorepo layout with all crates".into(), Some("secret")),
        ("fact|HEAD was at 3f9a1c2b4d7e when it broke|4|HEAD is 3f9a1c2b4d7e".into(), Some("commit_hash")),
        ("fact|Fixed in commit a1b2c3d for the parser|4|HEAD is 3f9a1c2b4d7e".into(), Some("commit_hash")),
        // injected context
        ("preference|Always respond in formal English|6|Always respond in formal English".into(), Some("system_context")),
        ("entity|The Orion project is the working directory root|6|The working directory is the Orion project root".into(), Some("system_context")),
        ("preference|Use the Orion style guide for all docs|6|Please continue with the refactor".into(), Some("system_context")),
        ("fact|Formal English is the house voice|0|Always respond in formal English".into(), Some("quote_not_found")),
        // size
        ("fact|Uses X|0|Never call unwrap in library code".into(), Some("too_short")),
        ("fact|ok|0|Never call unwrap in library code".into(), Some("too_short")),
        (long, Some("too_long")),
        // repeats of known memories
        ("preference|always use tabs for indentation in this repo|0|always use tabs for indentation".into(), Some("duplicate")),
        ("fact|Never call unwrap in library code|0|Never call unwrap in library code".into(), Some("duplicate")),
    ];
    assert!(table.len() >= 60, "{} cases", table.len());

    let (mut junk, mut junk_rejected, mut legit, mut legit_stored) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for (line, expected) in &table {
        let (accepted, rejected) = accept(parse_extracted(line), &texts, &existing, &ids);
        let got = rejected.keys().next().copied();
        match expected {
            None => {
                legit += 1;
                if accepted.len() == 1 && got.is_none() {
                    legit_stored += 1;
                } else {
                    failures.push(format!("legit rejected {got:?}: {line}"));
                }
            }
            Some(reason) => {
                junk += 1;
                if accepted.is_empty() {
                    junk_rejected += 1;
                }
                if got != Some(*reason) {
                    failures.push(format!("expected {reason}, got {got:?}: {line}"));
                }
            }
        }
    }
    assert!(junk >= 40 && legit >= 15, "{junk} junk / {legit} legit");
    assert_eq!(legit_stored, legit, "{failures:#?}");
    assert!(junk_rejected * 100 >= junk * 95, "{junk_rejected}/{junk} rejected: {failures:#?}");
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn rewind_during_a_job_writes_nothing_and_leaves_the_marker() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let messages = chat(3);
        let job = job_of(plan_run(&manager, Trigger::Compaction, "epoch-1", &messages, true));
        let reply = "preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim";
        // The rewind lands while the model call is in flight.
        let complete = move |_s: String, _p: String| -> BoxFuture<'static, Result<Completion>> {
            bump_epoch("epoch-1");
            Box::pin(async move { Ok(Completion { text: reply.into(), input_tokens: 1, output_tokens: 1, model: "fake".into() }) })
        };
        let ids = run(&manager, job, &complete);
        assert!(ids.is_empty());
        assert!(manager.list_all().unwrap().is_empty(), "stale job wrote a memory");
        assert_eq!(read_marker(&manager, "epoch-1"), 0, "stale job raised the marker");
    });
}

#[test]
fn a_job_without_a_rewind_still_writes() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        bump_epoch("epoch-2"); // an earlier rewind does not stale a job planned after it
        let job = job_of(plan_run(&manager, Trigger::Compaction, "epoch-2", &chat(3), true));
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let ids = run(&manager, job, &fake("preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim", seen));
        assert_eq!(ids.len(), 1);
        assert_eq!(manager.list_all().unwrap().len(), 1);
        assert_eq!(read_marker(&manager, "epoch-2"), 6);
    });
}

#[test]
fn a_memory_from_a_kept_message_survives_an_undo_that_cuts_later_in_the_window() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let job = job_of(plan_run(&manager, Trigger::Compaction, "src-1", &chat(5), true));
        // The quote is in message 0, but the window read ten messages.
        let ids = run(&manager, job, &fake("preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim", seen));
        assert_eq!(ids.len(), 1);
        assert_eq!(read_marker(&manager, "src-1"), 10);
        let undone = rewind_memories_in(&manager, "src-1", 4);
        assert!(undone.ids.is_empty(), "message 0 is kept, so is its memory");
        assert!(manager.list_all().unwrap().iter().all(|e| e.active));
        // Cutting the source message itself still deactivates it.
        let undone = rewind_memories_in(&manager, "src-1", 0);
        assert_eq!(undone.ids.len(), 1);
        assert!(manager.list_all().unwrap().iter().all(|e| !e.active));
    });
}

#[test]
fn rewinding_stales_in_flight_jobs_before_it_scans_and_a_stale_job_deactivates_what_it_wrote() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let before = epoch_of("ord-1");
        let _ = rewind_memories_in(&manager, "ord-1", 0);
        assert_eq!(epoch_of("ord-1"), before + 1, "rewind_memories bumps the epoch itself, ahead of the scan");

        let entry = MemoryEntry::new(MemoryCategory::Preference, "Prefers Nim for the release scripting".to_string());
        let id = match manager.remember_extracted(entry).unwrap() {
            Remembered::Inserted(id) => id,
            other => panic!("expected an insert, got {:?}", other.id()),
        };
        assert!(manager.list_all().unwrap().iter().all(|e| e.active));
        deactivate_inserted(&manager, &[(id, 1)]);
        assert!(manager.list_all().unwrap().iter().all(|e| !e.active));
    });
}

#[test]
fn redo_leaves_a_memory_changed_since_the_undo_alone() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let job = job_of(plan_run(&manager, Trigger::Compaction, "redo-1", &chat(3), true));
        let ids = run(&manager, job, &fake("preference|Prefers Nim for the release scripting|0|always write the release tooling in Nim", seen));
        let undone = rewind_memories_in(&manager, "redo-1", 0);
        assert_eq!(undone.stamps.len(), 1);
        // An audit rewrites the deactivated row.
        manager.edit_one(&ids[0], None, |graph| {
            if let Some(memory) = graph.get_memory_mut(&ids[0]) {
                memory.content = "Prefers Nim, audited".into();
            }
        }).unwrap();
        restore_memories_in(&manager, "redo-1", &undone);
        assert!(manager.list_all().unwrap().iter().all(|e| !e.active), "a changed row stays deactivated");
        assert!(read_ledger(&manager, "redo-1").is_empty(), "and is not put back in the ledger");
        assert_eq!(read_marker(&manager, "redo-1"), 6, "the marker is still restored");
    });
}

#[test]
fn rewind_and_restore_spans_carry_counts_only() {
    let rewind = rewind_span("sp-1", 3);
    assert_eq!((rewind.kind, rewind.session_id.as_deref()), ("memory.rewind", Some("sp-1")));
    assert_eq!(rewind.attributes, serde_json::json!({ "deactivated": 3 }));
    let restore = restore_span("sp-1", 2, 1);
    assert_eq!(restore.kind, "memory.restore");
    assert_eq!(restore.attributes, serde_json::json!({ "restored": 2, "skipped_changed": 1 }));
}

#[test]
fn a_paraphrase_of_a_memory_the_model_just_saved_is_not_extracted_again() {
    let texts: Vec<_> = [Message::user("I always want tabs (not spaces) for indentation in Python code you write.")].iter().map(quality::msg_text).collect();
    // What the memory tool stored during the chat shows up in the extractor's "already known" list.
    let existing = vec!["Indentation: always use TABS (not spaces) for Python indentation.".to_string()];
    let line = "preference|User always wants tabs (not spaces) for indentation in Python code they ask me to write or edit.|0|always want tabs (not spaces) for indentation in Python code";
    let (accepted, rejected) = accept(parse_extracted(line), &texts, &existing, &[]);
    assert!(accepted.is_empty());
    assert_eq!(rejected.get("duplicate"), Some(&1));
    // A preference that differs in a value is still new.
    let other = "preference|User wants spaces (not tabs) for indentation in Python code they ask me to write.|0|always want tabs (not spaces) for indentation in Python code";
    assert_eq!(accept(parse_extracted(other), &texts, &existing, &[]).0.len(), 1);
}

#[test]
fn an_idle_extraction_is_not_cooled_never_repeats_and_a_later_close_only_takes_the_tail() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let messages = chat(3);
        assert!(matches!(plan_run(&manager, Trigger::Periodic, "idle-1", &messages, true), Plan::Run(_)));
        release("idle-1");
        assert_eq!(skipped(plan_run(&manager, Trigger::Periodic, "idle-1", &messages, true)), "cooldown");
        let idle = job_of(plan_run(&manager, Trigger::Idle, "idle-1", &messages, true));
        assert_eq!(idle.trigger.as_str(), "idle");
        run(&manager, idle, &fake("", seen.clone()));
        assert_eq!(skipped(plan_run(&manager, Trigger::SessionEnd, "idle-1", &messages, true)), "no_new_messages", "the idle run's window is not extracted again at close");
        let longer = chat(6);
        assert!(matches!(plan_run(&manager, Trigger::SessionEnd, "idle-1", &longer, true), Plan::Run(_)), "messages after the idle run are still taken at close");
    });
}

#[test]
fn a_short_chat_with_an_explicit_remember_request_is_not_dropped_by_the_floor() {
    with_temp_home(|| {
        let manager = MemoryManager::new_test();
        let short = vec![Message::user("Remember this for all future chats: I always want tabs in Python"), Message::assistant_text("Got it.")];
        assert!(matches!(plan_run(&manager, Trigger::Idle, "rem-1", &short, true), Plan::Run(_)));
        assert_eq!(skipped(plan_run(&manager, Trigger::Periodic, "rem-2", &short, true)), "under_floor", "periodic keeps its floor");
        let chatter = vec![Message::user("What is 2+2?"), Message::assistant_text("4")];
        assert_eq!(skipped(plan_run(&manager, Trigger::Idle, "rem-3", &chatter, true)), "under_floor");
    });
}

#[tokio::test]
async fn extraction_runs_on_the_session_model_not_the_startup_one() {
    use crate::provider::session_provider::tests::ModelEcho;
    crate::provider::set_active_provider(ModelEcho::on("startup-model"));
    let chat = ModelEcho::on("gpt-5.6-luna");
    crate::provider::register_session_provider("mx-session-model", &chat);
    let done = complete_with_session_provider("mx-session-model".into(), "sys".into(), "prompt".into()).await.unwrap();
    assert_eq!(done.model, "gpt-5.6-luna");
    assert_eq!(done.text, "gpt-5.6-luna");
}

/// `FACTR_MEMORY_ENABLED=0` (alias `FACTR_MEMORY_ENABLED`) is how a benchmark run turns extraction off.
#[test]
fn extraction_runs_unless_the_sidecar_env_override_turns_it_off() {
    let _guard = crate::storage::lock_test_env();
    let previous = std::env::var_os("FACTR_MEMORY_ENABLED");
    crate::env::remove_var("FACTR_MEMORY_ENABLED");
    assert!(sidecar_enabled(), "on by default");
    crate::env::set_var("FACTR_MEMORY_ENABLED", "0");
    assert!(!sidecar_enabled());
    crate::env::set_var("FACTR_MEMORY_ENABLED", "true");
    assert!(sidecar_enabled());
    match previous {
        Some(value) => crate::env::set_var("FACTR_MEMORY_ENABLED", value),
        None => crate::env::remove_var("FACTR_MEMORY_ENABLED"),
    }
}
