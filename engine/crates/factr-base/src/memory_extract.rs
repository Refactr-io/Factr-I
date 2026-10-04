//! Automatic memory extraction (factr's, restored on the single store).
//!
//! A chat is turned into short fact/preference/correction/entity memories by one call to the
//! active provider. Triggers (all outside the gateway): every 12 fresh user turns, session end,
//! before compaction drops messages. Each trigger extracts only the messages after the session's
//! persisted `extracted_through` index, under a 200-char / 4-message floor, a 60 s per-session
//! cooldown (SessionEnd and Compaction are exempt), an in-flight claim taken at plan time, and one process-wide
//! aux-call permit. The window is built oldest-first from the marker and the marker moves only to
//! the last message actually included, so an oversized backlog is consumed across successive calls. It runs in a spawned task and never fails a
//! turn: errors become a `memory.extract` span.

use crate::memory::{MemoryCategory, MemoryEntry, MemoryManager};
use crate::memory_quality::{self as quality, Reject};
use crate::memory_store::Remembered;
use crate::message::{ContentBlock, Message, Role};
use crate::obs_sink::{self, Span};
use anyhow::{Result, anyhow};
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Fresh user turns between periodic extractions (upstream's interval).
const PERIODIC_INTERVAL: usize = 12;
/// Periodic runs look at no more than this many new messages.
const PERIODIC_MAX_MESSAGES: usize = 40;
/// Transcript cap per aux call (oldest messages first; the rest goes to the next window).
const MAX_TRANSCRIPT_CHARS: usize = 24_000;
const MIN_TRANSCRIPT_CHARS: usize = 200;
const MIN_MESSAGES: usize = 4;
const COOLDOWN: Duration = Duration::from_secs(60);
/// How long a chat may sit quiet (no turn running, no window attached) before it is treated as over:
/// the dropped-connection cleanup waits this long for a reconnect, and an open chat nobody is talking
/// in gets its idle extraction after the same wait. One duration for both.
pub const IDLE_GRACE: Duration = Duration::from_secs(30);
const EXISTING_LIMIT: usize = 80;
const EXISTING_CHARS: usize = 150;
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a job may wait for the shared aux-call permit.
const PERMIT_TIMEOUT: Duration = Duration::from_secs(120);
/// An in-flight claim older than this is treated as abandoned (the task died).
const CLAIM_TTL: Duration = Duration::from_secs(CALL_TIMEOUT.as_secs() + PERMIT_TIMEOUT.as_secs() + 30);
/// Windows one non-periodic job may consume in a row.
const MAX_WINDOWS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Periodic,
    SessionEnd,
    Compaction,
    /// The chat went quiet (or the user moved to another chat) without closing: same window as
    /// `SessionEnd`, but the session lives on, so its state is kept.
    Idle,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::SessionEnd => "session_end",
            Self::Compaction => "compaction",
            Self::Idle => "idle",
        }
    }
}

#[derive(Default)]
struct SessionState {
    turns: usize,
    /// When the last job finished (cooldown clock).
    last_done: Option<Instant>,
    /// When a planned job claimed the session; cleared when it finishes.
    claimed: Option<Instant>,
    /// Bumped when history is rewound; a job that started under an older epoch is stale.
    epoch: u64,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, SessionState>>> = LazyLock::new(Default::default);

/// One extraction/learning aux call at a time per process, so a local model is not hit in
/// parallel with the user's own turn. factr-learn learning takes the same permit.
static AUX_CALLS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

pub async fn aux_call_permit() -> tokio::sync::SemaphorePermit<'static> {
    AUX_CALLS.acquire().await.expect("aux semaphore is never closed")
}

/// The permit, or `None` if it did not free up within `wait` (background work must not queue forever).
pub async fn aux_call_permit_within(wait: Duration) -> Option<tokio::sync::SemaphorePermit<'static>> {
    tokio::time::timeout(wait, aux_call_permit()).await.ok()
}

/// Count a fresh user turn; true on every 12th (the caller then triggers a periodic run).
pub fn note_user_turn(session_id: &str) -> bool {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    let state = sessions.entry(session_id.to_string()).or_default();
    state.turns += 1;
    state.turns.is_multiple_of(PERIODIC_INTERVAL)
}

/// A session first seen in this process (attached, resumed or closed without a user turn here) that
/// has no `extracted_through` marker pre-dates extraction: stamp the marker at its current message
/// count, so only messages that arrive from now on are ever extracted. Once per session per process.
pub fn adopt_session(session_id: &str, total: usize) {
    {
        let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        if sessions.contains_key(session_id) {
            return;
        }
        sessions.insert(session_id.to_string(), SessionState::default());
    }
    let manager = MemoryManager::new();
    if matches!(manager.meta_get(&through_key(session_id)), Ok(None)) {
        let _ = manager.meta_set(&through_key(session_id), &total.to_string());
    }
}

/// The rewind path calls this: jobs already in flight over the rewound range become stale and write nothing.
pub fn bump_epoch(session_id: &str) {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    sessions.entry(session_id.to_string()).or_default().epoch += 1;
}

fn epoch_of(session_id: &str) -> u64 {
    SESSIONS.lock().unwrap_or_else(|p| p.into_inner()).get(session_id).map_or(0, |s| s.epoch)
}

/// Drop a closed session's counters (upstream's map grew without bound).
pub fn forget_session(session_id: &str) {
    crate::learn_signal::forget(session_id);
    SESSIONS.lock().unwrap_or_else(|p| p.into_inner()).remove(session_id);
}

fn in_flight(session_id: &str) -> bool {
    let sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    sessions.get(session_id).and_then(|s| s.claimed).is_some_and(|at| at.elapsed() < CLAIM_TTL)
}

fn cooldown_active(session_id: &str) -> bool {
    let sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    sessions.get(session_id).and_then(|s| s.last_done).is_some_and(|at| at.elapsed() < COOLDOWN)
}

/// Claim the session for a planned job (at plan time, so a second trigger sees it in flight).
fn claim(session_id: &str) {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    sessions.entry(session_id.to_string()).or_default().claimed = Some(Instant::now());
}

/// Release the claim and start the cooldown. Never recreates a session that was forgotten.
fn release(session_id: &str) {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(state) = sessions.get_mut(session_id) {
        state.claimed = None;
        state.last_done = Some(Instant::now());
    }
}

/// Whether automatic extraction (an LLM call on the active provider) may run: on unless
/// `FACTR_MEMORY_ENABLED` (`FACTR_MEMORY_ENABLED`) turns it off.
fn sidecar_enabled() -> bool {
    crate::factr_config::env_bool("FACTR_MEMORY_ENABLED").unwrap_or(true)
}

fn through_key(session_id: &str) -> String {
    format!("extracted_through:{session_id}")
}

fn extracted_through(manager: &MemoryManager, session_id: &str, total: usize) -> usize {
    let stored = manager
        .meta_get(&through_key(session_id))
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    // A shorter history than the marker means the session was rewound (undo/retry). Move the marker
    // to the new length: never reset to 0, which would re-extract the whole session. The rewind path
    // has already deactivated the memories from the undone turns (`rewind_memories`).
    if stored > total {
        let _ = manager.meta_set(&through_key(session_id), &total.to_string());
        total
    } else {
        stored
    }
}

fn ledger_key(session_id: &str) -> String {
    format!("extracted_ids:{session_id}")
}

/// Per session, the memories its extraction created and the message count each window read up to.
fn read_ledger(manager: &MemoryManager, session_id: &str) -> Vec<(String, usize)> {
    manager.meta_get(&ledger_key(session_id)).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}

fn write_ledger(manager: &MemoryManager, session_id: &str, ledger: &[(String, usize)]) {
    if let Ok(text) = serde_json::to_string(ledger) {
        let _ = manager.meta_set(&ledger_key(session_id), &text);
    }
}

/// `inserted` is (id, message count through the memory's verified source message).
fn record_inserted(manager: &MemoryManager, session_id: &str, inserted: &[(String, usize)]) {
    if inserted.is_empty() {
        return;
    }
    let mut ledger = read_ledger(manager, session_id);
    ledger.extend(inserted.iter().cloned());
    write_ledger(manager, session_id, &ledger);
}

fn deactivate_inserted(manager: &MemoryManager, inserted: &[(String, usize)]) {
    for (id, _) in inserted {
        set_active(manager, id, false);
    }
}

fn set_active(manager: &MemoryManager, id: &str, active: bool) {
    let _ = manager.edit_one(id, None, |graph| {
        if let Some(memory) = graph.get_memory_mut(id) {
            memory.active = active;
        }
    });
}

/// What a rewind took out of memory: the extraction marker as it was and the memories deactivated
/// (id, messages read when created). Kept in the redo entry so `restore_memories` can undo it.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UndoneMemories {
    pub marker: usize,
    pub ids: Vec<(String, usize)>,
    /// Each deactivated row as stored right after the deactivation. Redo reactivates a row only
    /// while it still matches (an audit or the user may have changed it since).
    #[serde(default)]
    pub stamps: Vec<(String, String)>,
}

/// Counts only: how many memories an undo deactivated.
fn rewind_span(session_id: &str, deactivated: usize) -> Span {
    Span::new("memory.rewind").session(session_id).attr("deactivated", deactivated)
}

/// Counts only: how many memories a redo reactivated and how many it left alone (changed since).
fn restore_span(session_id: &str, restored: usize, skipped: usize) -> Span {
    Span::new("memory.restore").session(session_id).attr("restored", restored).attr("skipped_changed", skipped)
}

fn stamp_of(manager: &MemoryManager, id: &str) -> Option<String> {
    let entry = manager.list_all().ok()?.into_iter().find(|e| e.id == id)?;
    serde_json::to_string(&entry).ok()
}

/// History was cut to `kept` stored messages: deactivate the memories extracted from beyond the cut
/// (through the one write path, `edit_one`). Bumps the session epoch before it scans, so jobs in
/// flight over the cut range cannot write after it. Call before the marker is lowered.
pub fn rewind_memories(session_id: &str, kept: usize) -> UndoneMemories {
    rewind_memories_in(&MemoryManager::new(), session_id, kept)
}

fn rewind_memories_in(manager: &MemoryManager, session_id: &str, kept: usize) -> UndoneMemories {
    // Stale the in-flight jobs first: one that writes after this scan would leave a memory from the
    // undone range alive and unledgered.
    bump_epoch(session_id);
    let marker = read_marker(manager, session_id);
    let (gone, left): (Vec<_>, Vec<_>) = read_ledger(manager, session_id).into_iter().partition(|(_, upto)| *upto > kept);
    for (id, _) in &gone {
        set_active(manager, id, false);
    }
    if !gone.is_empty() {
        write_ledger(manager, session_id, &left);
    }
    if !gone.is_empty() {
        obs_sink::emit(rewind_span(session_id, gone.len()));
    }
    let stamps = gone.iter().filter_map(|(id, _)| stamp_of(manager, id).map(|stamp| (id.clone(), stamp))).collect();
    UndoneMemories { marker, ids: gone, stamps }
}

/// Redo: reactivate what `rewind_memories` deactivated and put the marker back, so the restored
/// messages are not extracted a second time.
pub fn restore_memories(session_id: &str, undone: &UndoneMemories) {
    restore_memories_in(&MemoryManager::new(), session_id, undone);
}

fn restore_memories_in(manager: &MemoryManager, session_id: &str, undone: &UndoneMemories) {
    let mut restored = Vec::new();
    for (id, upto) in &undone.ids {
        // A row changed since the undo (audited, edited, reactivated by the user) is left as it is.
        let unchanged = undone.stamps.iter().find(|(sid, _)| sid == id).is_some_and(|(_, stamp)| stamp_of(manager, id).as_ref() == Some(stamp));
        if unchanged {
            set_active(manager, id, true);
            restored.push((id.clone(), *upto));
        }
    }
    if !undone.ids.is_empty() {
        obs_sink::emit(restore_span(session_id, restored.len(), undone.ids.len() - restored.len()));
    }
    if !restored.is_empty() {
        let mut ledger = read_ledger(manager, session_id);
        ledger.extend(restored);
        write_ledger(manager, session_id, &ledger);
    }
    let _ = manager.meta_set(&through_key(session_id), &undone.marker.to_string());
}

/// A memory as the model wrote it (`CATEGORY|CONTENT|MSG_INDEX|QUOTE`). Trust is never read from the
/// model: code derives it from where `quote` really occurs.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub category: String,
    pub content: String,
    /// Index of the message in the window shown to the model; `None` for the old 3-field format.
    pub msg_index: Option<usize>,
    pub quote: String,
}

/// One memory per line, other lines ignored. A line with fewer than four fields, or a non-numeric
/// index, still parses but has no `msg_index`, so it can never be verified and is rejected.
pub fn parse_extracted(response: &str) -> Vec<Extracted> {
    response
        .lines()
        .filter(|line| line.contains('|'))
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            (parts.len() >= 3).then(|| {
                let index = parts[2].trim().trim_matches(|c| c == '[' || c == ']' || c == '#').parse::<usize>().ok();
                let quote = parts.get(3).map(|q| q.trim()).unwrap_or("");
                Extracted {
                    category: parts[0].trim().to_lowercase(),
                    content: parts[1].trim().to_string(),
                    msg_index: index.filter(|_| !quote.is_empty()),
                    quote: quote.to_string(),
                }
            })
        })
        .filter(|m| !m.content.is_empty())
        .collect()
}

const SYSTEM_PROMPT: &str = r#"You are a memory extraction assistant. Extract important NEW learnings from the conversation that should be remembered for future sessions.

Categories (use EXACTLY one of these):
- fact: Technical facts about the codebase, architecture, patterns, dependencies, tools, environment
- preference: User preferences, workflow habits, UX expectations, coding style, conventions, how they want the assistant to behave
- correction: Mistakes that were corrected, bugs found and fixed, wrong assumptions, things the user corrected
- entity: Named entities worth tracking - people, projects, services, repos, teams

Categorization rules:
- If it describes what the USER WANTS or HOW THEY LIKE THINGS, it is "preference", not "fact"
- If it describes a BUG FIX or MISTAKE, it is "correction", not "fact"
- "fact" is for objective technical information about code/systems, not user behavior

IMPORTANT - Do NOT extract:
- Transient debugging details, compile errors, or intermediate build steps
- Specific commit hashes, git operations, or "changes were committed/pushed" details
- Line-by-line code changes like "X was updated to Y in file Z" - these belong in git history, not memory
- Self-evident project context (e.g., the project name, repo URL, language) that is already in the system prompt
- Redundant variations of information already known (check the "Already known" list carefully)

Quality bar: Only extract information that would ACTUALLY BE USEFUL if recalled in a future session on a different topic. Ask: "Would a developer benefit from knowing this weeks from now?"

For each memory, output in this format (one per line):
CATEGORY|CONTENT|MSG_INDEX|QUOTE

Where:
- CATEGORY is one of: fact, preference, correction, entity
- CONTENT is a concise statement (1-2 sentences max, under 200 characters preferred)
- MSG_INDEX is the number in the [#N] header of the message that states it
- QUOTE is a short exact quote (copied verbatim, no '|') from that message that supports the memory

Output ONLY the formatted lines, no other text. If no NEW memories worth extracting, output nothing."#;

fn system_prompt(existing: &[String]) -> String {
    let mut system = SYSTEM_PROMPT.to_string();
    if !existing.is_empty() {
        system.push_str("\n\nAlready known (do NOT re-extract these or close paraphrases):\n");
        for mem in existing.iter().take(EXISTING_LIMIT) {
            system.push_str("- ");
            system.push_str(crate::util::truncate_str(mem, EXISTING_CHARS));
            system.push('\n');
        }
    }
    system
}

/// Remove `<system-reminder>...</system-reminder>` blocks (an unclosed one runs to the end).
fn strip_system_reminders(text: &str) -> String {
    crate::memory_quality::split_reminders(text).0
}

/// One message as the model sees it: a `[#index] Role:` header (index within the window), then its text.
fn message_chunk(index: usize, msg: &Message) -> String {
    let role = match msg.role {
        Role::User => "User",
        Role::Assistant => "Assistant",
    };
    let mut body = String::new();
    for block in &msg.content {
        match block {
            ContentBlock::Text { text, .. } => {
                let text = strip_system_reminders(text);
                let text = text.trim();
                if !text.is_empty() {
                    body.push_str(text);
                    body.push('\n');
                }
            }
            ContentBlock::ToolUse { name, .. } => body.push_str(&format!("[Used tool: {name}]\n")),
            ContentBlock::ToolResult { content, .. } => {
                let preview = if content.len() > crate::memory_quality::TOOL_PREVIEW_BYTES {
                    format!("{}...", crate::util::truncate_str(content, crate::memory_quality::TOOL_PREVIEW_BYTES))
                } else {
                    content.clone()
                };
                body.push_str(&format!("[Result: {preview}]\n"));
            }
            ContentBlock::Image { .. } => body.push_str("[Image]\n"),
            ContentBlock::OpenAICompaction { .. } => body.push_str("[OpenAI native compaction]\n"),
            ContentBlock::Reasoning { .. }
            | ContentBlock::ReasoningTrace { .. }
            | ContentBlock::AnthropicThinking { .. }
            | ContentBlock::OpenAIReasoning { .. } => {}
        }
    }
    if body.is_empty() { String::new() } else { format!("[#{index}] {role}:\n{body}\n") }
}

/// The oldest-first window of `messages`: at most `max_messages` messages and `max_chars` chars.
/// Returns the transcript and how many messages it consumed (a message that would overflow the
/// cap is left for the next window, unless it is the first and gets truncated).
fn build_window(messages: &[Message], max_messages: usize, max_chars: usize) -> (String, usize) {
    let mut out = String::new();
    let mut total = 0usize;
    let mut consumed = 0usize;
    for (index, msg) in messages.iter().take(max_messages).enumerate() {
        let chunk = message_chunk(index, msg);
        if !chunk.is_empty() {
            let len = chunk.chars().count();
            if total + len > max_chars {
                if total == 0 {
                    out.extend(chunk.chars().take(max_chars));
                    consumed += 1;
                }
                break;
            }
            total += len;
            out.push_str(&chunk);
        }
        consumed += 1;
    }
    (out, consumed)
}

struct Job {
    trigger: Trigger,
    session_id: String,
    /// Absolute index of `messages[0]`.
    from: usize,
    /// Every message from `from` to the end of the planned range.
    messages: Vec<Message>,
    transcript: String,
    /// Last message the transcript actually includes: `extracted_through` becomes this on success.
    upto: usize,
    /// The session epoch when the job was planned.
    epoch: u64,
}

impl Job {
    fn window(&self) -> (usize, usize) {
        let max_messages = if self.trigger == Trigger::Periodic { PERIODIC_MAX_MESSAGES } else { usize::MAX };
        (max_messages, MAX_TRANSCRIPT_CHARS)
    }

    /// Rebuild the window so it starts at absolute index `start` (>= `from`).
    fn rewindow(&mut self, start: usize) {
        let skip = start.saturating_sub(self.from).min(self.messages.len());
        self.messages.drain(..skip);
        self.from += skip;
        let (max_messages, max_chars) = self.window();
        let (transcript, consumed) = build_window(&self.messages, max_messages, max_chars);
        self.transcript = transcript;
        self.upto = self.from + consumed;
    }

    fn drained(&self) -> bool {
        self.upto >= self.from + self.messages.len()
    }
}

/// An explicit save request in the window ("remember this", "from now on", "for all future chats"):
/// a one-line chat holding one must not be dropped for being short. Only session end, idle and
/// compaction runs use this; the periodic run keeps its floor.
pub fn asks_to_remember(transcript: &str) -> bool {
    let text = transcript.to_lowercase();
    ["remember", "from now on", "for all future", "always want", "never want", "don't forget", "do not forget"]
        .iter()
        .any(|cue| text.contains(cue))
}

enum Plan {
    Run(Job),
    Skip(&'static str),
}

/// Decide whether to run and build the transcript. `fetch(from)` returns messages `from..total`.
fn plan(
    manager: &MemoryManager,
    trigger: Trigger,
    session_id: &str,
    total: usize,
    enabled: bool,
    fetch: impl FnOnce(usize) -> Vec<Message>,
) -> Plan {
    if !enabled {
        return Plan::Skip("sidecar_off");
    }
    let through = extracted_through(manager, session_id, total);
    if total <= through {
        return Plan::Skip("no_new_messages");
    }
    // SessionEnd and Compaction are exempt: the tail (or the messages about to be cut) must never be
    // dropped. They queue behind any in-flight job on the aux permit and re-read the marker there,
    // so they neither repeat nor skip messages.
    if trigger == Trigger::Periodic {
        if in_flight(session_id) {
            return Plan::Skip("in_flight");
        }
        if cooldown_active(session_id) {
            return Plan::Skip("cooldown");
        }
    }
    let messages = fetch(through);
    let mut job = Job {
        trigger,
        session_id: session_id.to_string(),
        from: through,
        messages,
        transcript: String::new(),
        upto: through,
        epoch: epoch_of(session_id),
    };
    job.rewindow(through);
    let under = job.messages.len() < MIN_MESSAGES || job.transcript.chars().count() < MIN_TRANSCRIPT_CHARS;
    if under && !(trigger != Trigger::Periodic && asks_to_remember(&job.transcript)) {
        return Plan::Skip("under_floor");
    }
    claim(session_id);
    Plan::Run(job)
}

struct Completion {
    text: String,
    input_tokens: u64,
    output_tokens: u64,
    model: String,
}

type Complete<'a> = &'a (dyn Fn(String, String) -> BoxFuture<'static, Result<Completion>> + Send + Sync);

#[derive(Default)]
struct Outcome {
    existing_shown: usize,
    extracted: usize,
    written: usize,
    merged: usize,
    /// Rows this window created (not reinforced or merged), for the undo ledger: id and the message
    /// count through the verified source message (an undo that keeps that message keeps the memory).
    inserted: Vec<(String, usize)>,
    rejected: std::collections::BTreeMap<&'static str, usize>,
    /// A high-trust correction or preference was stored (see [`crate::learn_signal`]).
    correction: bool,
    input_tokens: u64,
    output_tokens: u64,
    model: String,
}

/// The gate: verify each memory's quote against the window (code decides trust), run the content
/// filters, drop repeats, cap per window. Returns the accepted memories with their trust and the
/// rejection counts by reason.
fn accept(
    extracted: Vec<Extracted>,
    texts: &[quality::MsgText],
    existing: &[String],
    identities: &[String],
) -> (Vec<(Extracted, crate::memory::TrustLevel)>, std::collections::BTreeMap<&'static str, usize>) {
    let context: String = texts.iter().map(|t| t.reminder.as_str()).collect::<Vec<_>>().join(" ");
    let ctx = quality::Ctx { identities, context: &context };
    let mut seen: std::collections::HashSet<String> = existing.iter().map(|e| quality::norm(e)).collect();
    let mut accepted = Vec::new();
    let mut rejected = std::collections::BTreeMap::new();
    for memory in extracted {
        let verdict = quality::verify(&memory.quote, memory.msg_index, texts)
            .and_then(|trust| quality::check(&memory.content, Some(&memory.quote), trust, &ctx).map(|()| trust))
            .and_then(|trust| if seen.contains(&quality::norm(&memory.content)) { Err(Reject::Duplicate) } else { Ok(trust) })
            // A paraphrase of a memory already shown to the model (including one the model itself
            // just saved with the memory tool: it is in the transcript, so it is in `existing`).
            .and_then(|trust| {
                let category = MemoryCategory::from_extracted(&memory.category);
                let repeats = |known: &String| crate::memory_store::same_fact(&category, &memory.content, known).is_some();
                if existing.iter().any(repeats) || accepted.iter().any(|(m, _): &(Extracted, _)| repeats(&m.content)) {
                    Err(Reject::Duplicate)
                } else {
                    Ok(trust)
                }
            })
            .and_then(|trust| if accepted.len() >= quality::MAX_PER_WINDOW { Err(Reject::OverCap) } else { Ok(trust) });
        match verdict {
            Ok(trust) => {
                seen.insert(quality::norm(&memory.content));
                accepted.push((memory, trust));
            }
            Err(reason) => *rejected.entry(reason.as_str()).or_insert(0) += 1,
        }
    }
    (accepted, rejected)
}

async fn execute(manager: &MemoryManager, job: &Job, complete: Complete<'_>) -> (Outcome, Result<Vec<String>>) {
    let mut outcome = Outcome::default();
    let result = async {
        // The SQLite parts run on the blocking pool, not on a tokio worker.
        let (m, transcript, session) = (manager.clone(), job.transcript.clone(), job.session_id.clone());
        let existing: Vec<String> = tokio::task::spawn_blocking(move || m.related_to(&transcript, EXISTING_LIMIT, Some(&session)))
            .await
            .map_err(|e| anyhow!("memory lookup task failed: {e}"))??
            .into_iter()
            .map(|e| e.content)
            .collect();
        outcome.existing_shown = existing.len();
        let completion = tokio::time::timeout(CALL_TIMEOUT, complete(system_prompt(&existing), job.transcript.clone()))
            .await
            .map_err(|_| anyhow!("extraction call timed out"))??;
        outcome.input_tokens = completion.input_tokens;
        outcome.output_tokens = completion.output_tokens;
        outcome.model = completion.model;
        let extracted = parse_extracted(&completion.text);
        outcome.extracted = extracted.len();
        let shown = job.upto.saturating_sub(job.from).min(job.messages.len());
        let texts: Vec<quality::MsgText> = job.messages[..shown].iter().map(quality::msg_text).collect();
        let (m, session, epoch) = (manager.clone(), job.session_id.clone(), job.epoch);
        let (stored, rejected, correction) = tokio::task::spawn_blocking(move || -> Result<(Vec<(Remembered, usize)>, _, bool)> {
            // Rewound while the model was running: the range is gone, write nothing.
            if epoch_of(&session) != epoch {
                return Ok((Vec::new(), Default::default(), false));
            }
            let (accepted, rejected) = accept(extracted, &texts, &existing, quality::identities());
            // What the user said about how they want things: the learning gate's cue to look.
            let correction = accepted.iter().any(|(m, trust)| *trust == crate::memory::TrustLevel::High && matches!(m.category.as_str(), "correction" | "preference"));
            let stored = accepted
                .into_iter()
                .map(|(memory, trust)| {
                    let entry = MemoryEntry::new(MemoryCategory::from_extracted(&memory.category), memory.content)
                        .with_source(&session)
                        .with_trust(trust);
                    let source = memory.msg_index.unwrap_or(0);
                    m.remember_extracted(entry).map(|remembered| (remembered, source))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((stored, rejected, correction))
        })
        .await
        .map_err(|e| anyhow!("memory write task failed: {e}"))??;
        outcome.rejected = rejected;
        outcome.correction = correction && !stored.is_empty();
        let mut ids = Vec::new();
        for (remembered, source) in stored {
            match remembered {
                Remembered::Inserted(_) => {
                    outcome.written += 1;
                    outcome.inserted.push((remembered.id().to_string(), job.from + source + 1));
                }
                Remembered::Reinforced(_) | Remembered::Merged { .. } => outcome.merged += 1,
            }
            ids.push(remembered.id().to_string());
        }
        Ok(ids)
    }
    .await;
    (outcome, result)
}

/// Lazy decay of stale unproven memories, at most once a day per process.
fn maybe_decay(manager: &MemoryManager) {
    static LAST_DAY: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let now = chrono::Utc::now();
    let day = now.timestamp() / 86_400;
    if LAST_DAY.swap(day, std::sync::atomic::Ordering::Relaxed) != day {
        match manager.decay_stale(now) {
            Ok(n) if n > 0 => obs_sink::emit(Span::new("memory.decay").attr("expired", n)),
            _ => {}
        }
    }
}

fn read_marker(manager: &MemoryManager, session_id: &str) -> usize {
    manager.meta_get(&through_key(session_id)).ok().flatten().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0)
}

/// Run one planned job to completion and record its spans. Returns the ids written or merged into.
///
/// The job waits for the aux permit (bounded), then re-reads the marker: a job that ran while it
/// waited may have consumed part of its range. A non-periodic job keeps going window by window until
/// its range is consumed, so an oversized backlog is never marked extracted without being read.
async fn run_job(manager: &MemoryManager, mut job: Job, complete: Complete<'_>) -> Vec<String> {
    let mut all_ids = Vec::new();
    let Some(_permit) = aux_call_permit_within(PERMIT_TIMEOUT).await else {
        obs_sink::emit(
            Span::new("memory.extract")
                .session(&job.session_id)
                .attr("trigger", job.trigger.as_str())
                .error("aux permit wait timed out"),
        );
        release(&job.session_id);
        return all_ids;
    };
    maybe_decay(manager);
    let end = job.from + job.messages.len();
    let marker = read_marker(manager, &job.session_id);
    if marker > job.from && marker <= end {
        job.rewindow(marker);
    }
    for window in 0..MAX_WINDOWS {
        if job.messages.is_empty() || job.transcript.chars().count() < MIN_TRANSCRIPT_CHARS {
            if window == 0 {
                skip(job.trigger, &job.session_id, "already_extracted");
            }
            break;
        }
        let started = Instant::now();
        let (outcome, result) = execute(manager, &job, complete).await;
        let mut span = Span::new("memory.extract")
            .session(&job.session_id)
            .attr("trigger", job.trigger.as_str())
            .attr("transcript_chars", job.transcript.chars().count())
            .attr("existing_shown", outcome.existing_shown)
            .attr("extracted", outcome.extracted)
            .attr("written", outcome.written)
            .attr("merged", outcome.merged)
            .attr("rejected", outcome.rejected.values().sum::<usize>())
            .attr("tokens", outcome.input_tokens + outcome.output_tokens)
            .attr("model", outcome.model.as_str())
            .tokens(outcome.input_tokens, outcome.output_tokens)
            .took_ms(started.elapsed().as_millis() as u64);
        for (reason, n) in &outcome.rejected {
            span = span.attr(&format!("rejected_{reason}"), *n);
        }
        if result.is_ok() && epoch_of(&job.session_id) != job.epoch {
            // Stale: leave the marker where the rewind put it. A write that landed before the epoch
            // moved is from the rewound range and is deactivated here (it is not in the ledger).
            deactivate_inserted(manager, &outcome.inserted);
            obs_sink::emit(
                Span::new("memory.skip")
                    .session(&job.session_id)
                    .attr("trigger", job.trigger.as_str())
                    .attr("reason", "stale_epoch")
                    .attr("extracted", outcome.extracted),
            );
            break;
        }
        let ok = match result {
            Ok(ids) => {
                let _ = manager.meta_set(&through_key(&job.session_id), &job.upto.to_string());
                record_inserted(manager, &job.session_id, &outcome.inserted);
                // A closed session must not get a fresh injected-id entry (the map is pruned on close).
                if job.trigger != Trigger::SessionEnd {
                    crate::memory::mark_memories_known(&job.session_id, &ids, "extracted from this session");
                    if outcome.correction {
                        crate::learn_signal::note(&job.session_id);
                    }
                }
                all_ids.extend(ids);
                true
            }
            Err(error) => {
                span = span.error(error.to_string());
                false
            }
        };
        obs_sink::emit(span);
        if !ok || job.trigger == Trigger::Periodic || job.drained() {
            break;
        }
        job.rewindow(job.upto);
    }
    release(&job.session_id);
    all_ids
}

async fn complete_with_session_provider(session_id: String, system: String, prompt: String) -> Result<Completion> {
    // The chat's own model, unless `FACTR_MEMORY_MODEL` or the user's auxiliary slot names one (never a hidden default).
    let provider = crate::provider::session_provider_fork(&session_id).ok_or_else(|| anyhow!("no active provider"))?;
    let chosen = crate::factr_config::env_text("FACTR_MEMORY_MODEL")
        .or_else(|| crate::factr_config::aux_model(crate::factr_config::AuxConsumer::MemoryExtraction));
    if let Some(model) = chosen.as_deref() {
        provider.set_model(model)?;
    }
    let model = provider.model();
    let done = provider.complete_simple_with_usage(&prompt, &system).await?;
    let (input_tokens, output_tokens) = done.usage.map_or((0, 0), |u| (u.input, u.output));
    Ok(Completion { text: done.text, input_tokens, output_tokens, model })
}

fn skip(trigger: Trigger, session_id: &str, reason: &'static str) {
    obs_sink::emit(Span::new("memory.skip").session(session_id).attr("trigger", trigger.as_str()).attr("reason", reason));
}

/// Plan an extraction of the session's messages `..total` and, if it should run, spawn it.
/// `fetch(from)` returns messages `from..total`. Never blocks and never fails the caller.
pub fn spawn(
    trigger: Trigger,
    session_id: &str,
    working_dir: Option<&str>,
    total: usize,
    fetch: impl FnOnce(usize) -> Vec<Message>,
) {
    let manager = match working_dir {
        Some(dir) if !dir.trim().is_empty() => MemoryManager::new().with_project_dir(dir),
        _ => MemoryManager::new(),
    };
    match plan(&manager, trigger, session_id, total, sidecar_enabled(), fetch) {
        Plan::Skip(reason) => skip(trigger, session_id, reason),
        Plan::Run(job) => {
            let session = job.session_id.clone();
            let work = async move {
                let complete = move |system, prompt| -> BoxFuture<'static, Result<Completion>> {
                    Box::pin(complete_with_session_provider(session.clone(), system, prompt))
                };
                run_job(&manager, job, &complete).await;
            };
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(work);
            } else {
                std::thread::spawn(move || {
                    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                        runtime.block_on(work);
                    }
                });
            }
        }
    }
}

/// Session-end extraction for a chat closed through the gateway (no live agent: reads the saved
/// session). Same path as the disconnect cleanup, so the `extracted_through` marker keeps a later
/// disconnect from extracting the same window; the session's counters are left for that cleanup
/// to forget (forgetting here would drop an in-flight claim). Spawned, never blocks.
pub fn extract_session_end(session_id: &str) {
    extract_saved(Trigger::SessionEnd, session_id);
}

/// Extraction for an open chat that has been quiet for [`IDLE_GRACE`] (the user switched to another
/// chat, or walked away): the saved session's unextracted tail, with the session left running.
/// Marker-guarded like every trigger, so a later close, disconnect or periodic run never repeats it.
pub fn extract_idle(session_id: &str) {
    extract_saved(Trigger::Idle, session_id);
}

fn extract_saved(trigger: Trigger, session_id: &str) {
    if !crate::config::memory_enabled() {
        return;
    }
    let Ok(session) = crate::session::Session::load(session_id) else { return };
    adopt_session(session_id, session.messages.len());
    spawn(trigger, session_id, session.working_dir.as_deref(), session.messages.len(), |from| {
        session.messages[from..].iter().map(|m| m.to_message()).collect()
    });
}

#[cfg(test)]
#[path = "memory_extract_tests.rs"]
mod tests;
