//! Automatic learning (the factr-learn loop) for chats served by this gateway.
//!
//! Ports learning agent's `reviewAutoRefine` / `_maybeAutoRefine` (`refinement.ts`, `agent-session.ts`),
//! with one difference: factr-learn asks its gate after every N assistant messages; here the gate is asked
//! only when the transcript holds a [`Signal`] (the user instructed or corrected, a tool failed and
//! was retried differently, the loop guard or auto-verify fired, a compaction happened, or memory
//! extraction stored a correction) and a cooldown has elapsed. The gate and the refine pass read the
//! window around the signals, not the whole transcript. `turn_interval` is only the ceiling after
//! which a signal-less stretch is dropped so the window cannot grow without bound. One lightweight
//! model call ("the gate") decides whether the window is worth a `/refine` pass; a "no" costs exactly
//! that one call and restarts the window, same as a "yes". A "yes" runs the existing `/refine` (prompt, skill and
//! subagent entries; fact-type memories are factr extraction's job) exactly once - the same path
//! the interactive `/refine` command and the model-callable `refine` tool use, so there is one
//! learning pipeline, not two. Every step emits a `learning.*` span.

use crate::rpc::Conn;
use factr_base::obs_sink::{Span, emit};
use serde_json::Value;
use factr_learn::refine::{GATE_TRANSCRIPT_CHARS, Turn, parse_json_object, transcript};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct Learning {
    /// Assistant messages since the last auto-refine review before the gate is asked again
    /// (factr-learn default: 25; `settings.autoRefine.turnInterval`).
    pub turn_interval: usize,
    /// Minimum time between two gate calls, regardless of the count
    /// (factr-learn default: 20 minutes; `settings.autoRefine.cooldownMs`).
    pub cooldown: Duration,
}

/// factr-learn's `AutoRefineReason`: why a review is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Trigger {
    TurnInterval,
    /// A context compaction happened: reviewed whatever the count, once the cooldown allows.
    Compact,
    /// The session is closing: a review that is due runs before it goes.
    Dispose,
}

impl Trigger {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Trigger::TurnInterval => "turn_interval",
            Trigger::Compact => "compact",
            Trigger::Dispose => "dispose",
        }
    }
}

/// A gate call that is due: how many assistant messages it covers, why it runs and what cued it.
#[derive(Clone, Debug)]
pub(crate) struct GateDue {
    pub assistants: usize,
    pub trigger: Trigger,
    pub signals: Vec<Signal>,
}

/// Why a review is worth a look. Only these are cues: a count of messages is not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SignalKind {
    /// The user asked to remember something, or corrected the assistant.
    UserInstruction,
    /// The repeat guard blocked or warned about an identical call.
    LoopGuard,
    /// Auto-verify ran the project's tests and they failed.
    AutoVerify,
    /// A tool failed and the same tool then succeeded with other arguments.
    ToolRetry,
    /// The context was compacted.
    Compaction,
    /// Memory extraction stored a correction or preference ([`factr_base::learn_signal`]).
    ExtractionCorrection,
}

impl SignalKind {
    fn as_str(self) -> &'static str {
        match self {
            SignalKind::UserInstruction => "user_instruction",
            SignalKind::LoopGuard => "loop_guard",
            SignalKind::AutoVerify => "auto_verify",
            SignalKind::ToolRetry => "tool_retry",
            SignalKind::Compaction => "compaction",
            SignalKind::ExtractionCorrection => "extraction_correction",
        }
    }
}

/// A cue at history row `index`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Signal {
    pub index: usize,
    pub kind: SignalKind,
}

/// The kinds of `signals`, once each, for a prompt line or a span.
fn kinds(signals: &[Signal]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for s in signals {
        if !seen.contains(&s.kind.as_str()) {
            seen.push(s.kind.as_str());
        }
    }
    seen.join(",")
}

/// A signal's window starts this many rows before it (what led to it: the step that provoked it)...
const WINDOW_BEFORE: usize = 2;
/// ...and ends this many after (the response to it and the tool results that followed).
const WINDOW_AFTER: usize = 3;
/// A failed tool call counts as retried when the same tool succeeds with other arguments within this
/// many rows: its own result plus two more call/result pairs.
const RETRY_ROWS: usize = 6;
/// The most characters of window the gate and the refine pass read: about 4,000 tokens at the usual
/// four characters a token, a fifth of factr-learn's 80k-character refine transcript (still what an
/// explicit `/refine` reads), enough for a handful of signal windows.
pub(crate) const SIGNAL_WINDOW_CHARS: usize = 16_000;

/// Cues that mark a user reply as a correction of the assistant (the "remember" cues live with the
/// extractor, [`factr_base::memory_extract::asks_to_remember`]). A bare "no" and "stop" are not
/// cues: they answer questions and interrupt work far more often than they correct it.
const CORRECTION_CUES: [&str; 6] = ["wrong", "don't", "never", "not that", "i said", "i meant"];

/// What the user wrote in a row, without the engine's reminders, lowercased with curly apostrophes
/// made straight.
fn said(text: &str) -> String {
    let (clean, _) = factr_base::memory_quality::split_reminders(text);
    clean.trim().to_lowercase().replace('\u{2019}', "'")
}

/// `text` begins with the word or phrase `cue` (not merely a word that starts with it).
fn leads_with(text: &str, cue: &str) -> bool {
    text.strip_prefix(cue).is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '\''))
}

/// `lower` (a user reply to the assistant) corrects it: a cue leads the reply (after an opening
/// "no"), or the reply is a single clause that carries one. "instead" counts the same way, but only
/// as the adverb that names a replacement ("use serde instead", "instead, use serde"), never as
/// "instead of", which describes the task. A cue in the middle of a longer request is part of the
/// task ("refactor the parser, but don't change the API"), not a correction.
fn corrects(lower: &str) -> bool {
    let opening = lower.strip_prefix("no").filter(|rest| rest.starts_with([',', ' ', '.', '!'])).map_or(lower, |rest| {
        rest.trim_start_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
    });
    let one_clause = !lower.trim_end_matches(['.', '!', '?']).contains(['.', '!', '?', ',', ';', ':', '\n']);
    let words: Vec<&str> = lower.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()).collect();
    let spaced = format!(" {} ", words.join(" "));
    CORRECTION_CUES.iter().any(|cue| leads_with(opening, cue) || (one_clause && spaced.contains(&format!(" {cue} "))))
        || (leads_with(opening, "instead") && !leads_with(opening, "instead of"))
        || (one_clause && words.last() == Some(&"instead"))
}

/// The user's message at `i` answers the assistant: the nearest earlier message (tool rows and
/// reminder-only rows aside) is the assistant's.
fn answers_assistant(raw: &[Value], i: usize) -> bool {
    raw[..i]
        .iter()
        .rev()
        .find_map(|r| match r["role"].as_str() {
            Some("assistant") => Some(true),
            Some("user") if !said(r["content"].as_str().unwrap_or_default()).is_empty() => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

/// The user's message at `i` asks to remember something, or corrects the assistant's last turn.
fn instructs(raw: &[Value], i: usize) -> bool {
    let lower = said(raw[i]["content"].as_str().unwrap_or_default());
    factr_base::memory_extract::asks_to_remember(&lower) || (corrects(&lower) && answers_assistant(raw, i))
}

/// A tool row the engine itself marked as failed. The history rows drop the protocol's `is_error`
/// flag, so this reads the two markers the engine writes: a tool that returned an error is stored as
/// `Error: <reason>` (agent/turn_loops.rs), and a shell command that exited non-zero ends with
/// `Exit code: <n>` (tool/bash.rs). Tool output that merely mentions an error is not a failure.
fn tool_failed(text: &str) -> bool {
    text.starts_with("Error: ")
        || text.trim_end().rsplit_once("\n\nExit code: ").is_some_and(|(_, code)| code.parse::<i64>().is_ok_and(|c| c != 0))
}

/// Whether the failed tool row at `i` is followed, within [`RETRY_ROWS`], by the same tool succeeding
/// with other arguments.
fn retried(raw: &[Value], i: usize) -> bool {
    let (name, input) = (&raw[i]["tool_data"]["name"], &raw[i]["tool_data"]["input"]);
    !name.is_null()
        && raw[i + 1..raw.len().min(i + 1 + RETRY_ROWS)].iter().any(|r| {
            r["role"] == "tool" && r["tool_data"]["name"] == *name && r["tool_data"]["input"] != *input && !tool_failed(r["content"].as_str().unwrap_or_default())
        })
}

/// The cues in `raw[from..]` (the rows since the watermark). `compact_pending` is a compaction not
/// yet reviewed and `extraction_hits` the corrections extraction stored; both point at the newest row.
pub(crate) fn signals(raw: &[Value], from: usize, compact_pending: bool, extraction_hits: usize) -> Vec<Signal> {
    let mut out = Vec::new();
    for (index, row) in raw.iter().enumerate().skip(from) {
        let text = row["content"].as_str().unwrap_or_default();
        let kind = match row["role"].as_str() {
            Some("user") if text.contains("You are repeating yourself") => Some(SignalKind::LoopGuard),
            Some("user") if text.contains("Auto-verify:") && text.contains("failed") => Some(SignalKind::AutoVerify),
            Some("user") if instructs(raw, index) => Some(SignalKind::UserInstruction),
            Some("tool") if text.starts_with("blocked: identical call repeated") => Some(SignalKind::LoopGuard),
            Some("tool") if tool_failed(text) && retried(raw, index) => Some(SignalKind::ToolRetry),
            _ => None,
        };
        out.extend(kind.map(|kind| Signal { index, kind }));
    }
    let newest = raw.len().saturating_sub(1);
    if compact_pending {
        out.push(Signal { index: newest, kind: SignalKind::Compaction });
    }
    if extraction_hits > 0 {
        out.push(Signal { index: newest, kind: SignalKind::ExtractionCorrection });
    }
    out
}

/// The transcript the gate and the refine pass read: the rows around every signal (unioned), tools
/// reduced to one line each, the newest [`SIGNAL_WINDOW_CHARS`] kept.
pub(crate) fn window(raw: &[Value], from: usize, signals: &[Signal]) -> Vec<Turn> {
    let Some(last) = raw.len().checked_sub(1) else { return Vec::new() };
    let mut rows = std::collections::BTreeSet::new();
    for s in signals {
        rows.extend(s.index.saturating_sub(WINDOW_BEFORE).max(from)..=(s.index + WINDOW_AFTER).min(last));
    }
    let mut used = 0;
    let mut turns: Vec<Turn> = compact_tools(rows.iter().map(|&i| &raw[i]))
        .into_iter()
        .rev()
        .take_while(|t| {
            used += t.text.chars().count();
            used <= SIGNAL_WINDOW_CHARS
        })
        .collect();
    turns.reverse();
    turns
}

fn window_chars(turns: &[Turn]) -> u64 {
    turns.iter().map(|t| t.text.chars().count() as u64).sum()
}

/// Assistant messages since `seen` (the engine's history has one `assistant` row per model
/// response, so a tool-heavy turn yields many): the span's measure of how much a review covers.
fn assistants_since(raw: &[Value], seen: usize) -> usize {
    raw.iter().skip(seen).filter(|m| m["role"] == "assistant").count()
}

/// Why no review is due: the `learning.skip` reason, the assistant count, and whether the
/// watermark moved past a signal-less stretch.
#[derive(Debug)]
pub(crate) struct Skip {
    pub reason: &'static str,
    pub assistants: usize,
    pub advanced: bool,
}

/// Whether a review is due for `session`, given its history `raw`: `Ok` with what to review, or
/// `Err` with why not. A review needs a [`Signal`] in the rows since the watermark and an elapsed
/// cooldown. `compact_pending` is a compaction not yet reviewed, `extraction_hits` the corrections
/// extraction stored since the last review. With no signal and `turn_interval` assistant messages
/// behind it, the stretch is dropped (the watermark moves) so the window never grows without bound.
pub(crate) fn due(
    store: &factr_learn::entries::EntryStore,
    session: &str,
    learning: &Learning,
    raw: &[Value],
    trigger: Trigger,
    compact_pending: bool,
    extraction_hits: usize,
    now: i64,
) -> Result<GateDue, Skip> {
    // An undo or rewind can leave fewer messages than the watermark.
    let seen = store.watermark(session).min(raw.len());
    let assistants = assistants_since(raw, seen);
    let skip = |reason, advanced| Skip { reason, assistants, advanced };
    let signals = signals(raw, seen, compact_pending, extraction_hits);
    if signals.is_empty() {
        let advanced = assistants >= learning.turn_interval && store.set_watermark(session, raw.len()).is_ok();
        return Err(skip("no_signal", advanced));
    }
    let cooldown_ms = learning.cooldown.as_millis() as i64;
    // A signal is a reason on its own: only the cooldown can still say no.
    match store.learn_checkpoint(session, assistants, learning.turn_interval, cooldown_ms, now, true) {
        Ok(Some(_)) => Ok(GateDue { assistants, trigger: if compact_pending { Trigger::Compact } else { trigger }, signals }),
        Ok(None) => Err(skip("cooldown", false)),
        Err(_) => Err(skip("store_error", false)),
    }
}

/// One learning model call, taking the process-wide aux permit that memory extraction also takes,
/// so the two never hit a (local) model together or in parallel with the user's own turn.
///
/// Both the permit wait and the call are bounded ([`AUX_TIMEOUT`]): a user's `/refine` must not sit
/// behind background work forever, and one hung provider call must not hold the slot indefinitely.
pub(crate) async fn aux_complete(
    complete: &crate::Complete,
    system: String,
    user: String,
) -> anyhow::Result<factr_provider_core::SimpleCompletion> {
    aux_complete_within(complete, system, user, AUX_TIMEOUT).await
}

/// [`aux_complete`] for `session`: the call runs on the chat's own model (see
/// `factr_base::provider::session_base`) unless Settings names an auxiliary one.
pub(crate) async fn aux_complete_for(
    complete: &crate::Complete,
    session: &str,
    system: String,
    user: String,
) -> anyhow::Result<factr_provider_core::SimpleCompletion> {
    factr_base::provider::with_aux_session(session, aux_complete(complete, system, user)).await
}

async fn aux_complete_within(
    complete: &crate::Complete,
    system: String,
    user: String,
    limit: Duration,
) -> anyhow::Result<factr_provider_core::SimpleCompletion> {
    let _permit = factr_base::memory_extract::aux_call_permit_within(limit)
        .await
        .ok_or_else(|| anyhow::anyhow!("timed out waiting for the model slot (background work is using it)"))?;
    tokio::time::timeout(limit, complete(system, user))
        .await
        .map_err(|_| anyhow::anyhow!("the model call timed out"))?
}

/// How long an aux call may wait for the shared permit, and then run.
pub(crate) const AUX_TIMEOUT: Duration = Duration::from_secs(120);

/// Run synchronous SQLite work without stalling a tokio worker. Only a multi-thread runtime can
/// hand the worker off; on a current-thread one (tests, the fallback thread) it just runs inline.
pub(crate) fn blocking<T>(f: impl FnOnce() -> T) -> T {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// A short class for a rejected proposal. The error text quotes entry titles and contents, so the
/// span records the class only.
fn error_class(err: &anyhow::Error) -> &'static str {
    let text = format!("{err:#}");
    if text.contains("no_evidence") {
        "no_evidence"
    } else if text.contains("no durable lesson") {
        "no_durable_lesson"
    } else if text.contains("not valid JSON") {
        "invalid_json"
    } else if text.contains("exceeds the limit") {
        "too_many_edits"
    } else if text.contains("rejected:") {
        "gate_rejected"
    } else {
        "store_error"
    }
}

/// A `learning.*` span for one model call: its tokens, duration and error, if any.
fn call_span(
    kind: &'static str,
    session: &str,
    trigger: Trigger,
    reply: &anyhow::Result<factr_provider_core::SimpleCompletion>,
    started: i64,
) -> Span {
    let mut span = Span::new(kind).session(session).attr("trigger", trigger.as_str());
    span = span.took_ms(crate::observability::now().saturating_sub(started).max(0) as u64);
    match reply {
        Ok(done) => {
            if let Some(u) = done.usage {
                span = span.tokens(u.input, u.output);
            }
            span
        }
        Err(err) => span.error(err.to_string()),
    }
}

/// `config.get learning.enabled`: factr-learn's auto-refine switch (on by default).
pub(crate) fn learning_enabled(home: &str) -> bool {
    factr_learn::entries::EntryStore::open_cached(std::path::Path::new(home)).map(|s| s.learning_enabled()).unwrap_or(true)
}

/// `config.set learning.enabled`: persist the switch (bool or "on"/"off"/"true"/"false").
pub(crate) fn set_learning_enabled(home: &str, value: &serde_json::Value) -> anyhow::Result<bool> {
    let on = value.as_bool().unwrap_or_else(|| {
        !matches!(value.as_str().unwrap_or("true").trim().to_ascii_lowercase().as_str(), "false" | "off" | "0" | "no")
    });
    factr_learn::entries::EntryStore::open_cached(std::path::Path::new(home))?
        .set_setting("learning.enabled", if on { "true" } else { "false" })?;
    Ok(on)
}

const MAX_TOOL_LINES: usize = 40;

/// Tool rows carry whole outputs; factr-learn's transcript shows `[Tool result (name, error)]` per call.
/// Reduce each to one line, `ok: <tool> <first line>` or `fail: ...` (first line <= 120 chars), and
/// keep the newest [`MAX_TOOL_LINES`] so the gate sees outcomes, not file dumps. The tool's name is
/// `tool_data.name`; success is the engine's own failure marker ([`tool_failed`]). Only user turns feed the apply gate.
fn compact_tools<'a>(rows: impl Iterator<Item = &'a serde_json::Value> + Clone) -> Vec<Turn> {
    let mut left = rows.clone().filter(|m| m["role"] == "tool").count().saturating_sub(MAX_TOOL_LINES);
    rows.filter_map(|m| {
            let text = m["content"].as_str().unwrap_or_default();
            if m["role"] != "tool" {
                return Some(Turn { role: m["role"].as_str().unwrap_or_default().to_string(), text: text.to_string() });
            }
            if left > 0 {
                left -= 1;
                return None;
            }
            let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
            let line: String = first.chars().take(120).collect();
            let name = m["tool_data"]["name"].as_str().filter(|n| !n.is_empty()).map(|n| format!("{n} ")).unwrap_or_default();
            Some(Turn { role: "tool".into(), text: format!("{} {name}{line}", if tool_failed(text) { "fail:" } else { "ok:" }) })
        })
        .collect()
}

/// factr-learn's `AUTO_REFINE_REVIEW_SYSTEM_PROMPT` / `parseAutoRefineReview`: one
/// cheap call that decides whether a `/refine` pass is worth its cost.
struct GateReview {
    should_refine: bool,
    rationale: Option<String>,
    instructions: Option<String>,
}

/// factr-learn's `autoRefineInstructions`: what the approving gate saw (its rationale and
/// instructions) is handed to the refine pass, which would otherwise start blind.
fn approved_instructions(trigger: Trigger, review: &GateReview) -> String {
    let mut text = format!(
        "Automatic refine review triggered by {}. Only create/update/delete entries if there is clear evidence \
         that should help this session or future ones; prefer an empty edits array over speculative or one-off \
         entries. Do not promote anything global unless explicitly requested.",
        trigger.as_str()
    );
    if let Some(r) = &review.rationale {
        text.push_str(&format!(" Reviewer rationale: {r}"));
    }
    if let Some(i) = &review.instructions {
        text.push_str(&format!("\nReviewer instructions: {i}"));
    }
    text
}

/// The gate already found a lesson in this exact window, so an empty or unparsable proposal is a
/// miss to correct once, not a result (factr-learn records nothing either, but its reviewer is a stronger model).
const APPROVED_RETRY: &str = "\n\nThe reviewer found a durable lesson in this conversation, so your previous reply \
    (empty or not valid JSON) was wrong. Propose the edit the reviewer described: a `prompt` or `subagent` \
    entry (or a `skill` where offered) for a reusable convention, procedure or delegation. Reply with the JSON \
    object only.";

/// An edit whose quotes are not in the window: one correction, the same way.
const EVIDENCE_RETRY: &str = "\n\nYour previous reply was rejected: every edit must carry `evidence`, the user's or \
    assistant's exact words copied from the conversation (never tool output). Quote the user's exact words and \
    reply with the JSON object only.";

/// The refine pass for an approving gate: one call, plus one corrective retry when it comes back
/// without an applicable edit. `record` sees every model call (reply and start time).
/// The outer `Err` is a failed model call (transient: the checkpoint must be retried); the inner
/// result is what applying the proposal came to (final: nothing more to gain from this window).
async fn refine_approved(
    complete: &crate::Complete,
    store: &factr_learn::entries::EntryStore,
    session: &str,
    fresh: &[Turn],
    due: &GateDue,
    review: &GateReview,
    record: &mut (dyn FnMut(&anyhow::Result<factr_provider_core::SimpleCompletion>, i64) + Send),
) -> anyhow::Result<anyhow::Result<factr_learn::refine::RefineOutcome>> {
    let mut instructions = approved_instructions(due.trigger, review);
    let mut attempt = 0;
    loop {
        let (system, user) = blocking(|| factr_learn::refine::build_request(store, session, fresh, Some(&instructions), false));
        let started = crate::observability::now();
        let reply = aux_complete_for(complete, session, system, user).await;
        record(&reply, started);
        emit(call_span("learning.refine", session, due.trigger, &reply, started).attr("attempt", attempt as u64));
        let text = reply?.text;
        let applied = blocking(|| factr_learn::refine::apply_learned(store, session, &text, false, "auto", fresh));
        let span = Span::new("learning.apply").session(session).attr("trigger", due.trigger.as_str());
        emit(match &applied {
            Ok(done) => span
                .attr("approved", true)
                .attr("changeset", done.changeset_id.as_str())
                .attr("edits", serde_json::to_value(&done.by_kind).unwrap_or_default())
                .attr("created", done.created.len() as u64)
                .attr("updated", done.updated.len() as u64)
                .attr("deleted", done.deleted.len() as u64),
            Err(err) => span.attr("approved", false).attr("created", 0u64).attr("updated", 0u64).attr("deleted", 0u64).attr("error_class", error_class(err)),
        });
        match applied {
            Err(err) if attempt == 0 && error_class(&err) == "no_evidence" => {
                attempt += 1;
                instructions.push_str(EVIDENCE_RETRY);
            }
            Err(err) if attempt == 0 && (format!("{err:#}").contains("no durable lesson") || format!("{err:#}").contains("not valid JSON")) => {
                attempt += 1;
                instructions.push_str(APPROVED_RETRY);
            }
            done => return Ok(done),
        }
    }
}

fn gate_request(store: &factr_learn::entries::EntryStore, session: &str, due: &GateDue, fresh: &[Turn]) -> (String, String) {
    let system = "You are this agent's automatic /refine review gate. Decide whether this checkpoint should run \
                  /refine. Auto /refine writes local Continual Harness state by default, so approve when the \
                  trajectory contains evidence useful to this session's future turns. Reject one-off noise, \
                  unsupported hypotheses, and transient tool output. Ask for global refinement only for durable \
                  cross-session lessons or explicitly project-qualified lessons likely to be reused in future \
                  sessions. Return JSON only: {\"shouldRefine\": true|false, \"rationale\": <short reason>, \
                  \"instructions\": <optional concise instructions for /refine if shouldRefine is true>}."
        .to_string();
    let user = format!(
        "<trigger>\n{}; {} assistant turns since the last auto-refine review; signals: {}\n</trigger>\n\
         <current_harness_state>\n{}\n</current_harness_state>\n\n\
         <refinement_history>\n{}\n</refinement_history>\n\n\
         <conversation>\n{}\n</conversation>\n\n\
         Return shouldRefine=true when the trajectory contains evidence useful to this session's future turns. \
         Prefer local harness edits for current task progress, temporary blockers, and current-run coordination.",
        due.trigger.as_str(),
        due.assistants,
        kinds(&due.signals),
        factr_learn::refine::overview(store, session),
        factr_learn::refine::history(store, session),
        transcript(fresh, GATE_TRANSCRIPT_CHARS)
    );
    (system, user)
}

fn parse_gate_review(reply: &str) -> GateReview {
    let Some(value) = parse_json_object(reply) else {
        return GateReview { should_refine: false, rationale: None, instructions: None };
    };
    GateReview {
        should_refine: value["shouldRefine"].as_bool().unwrap_or(false),
        rationale: value["rationale"].as_str().map(str::to_string),
        instructions: value["instructions"].as_str().map(str::to_string),
    }
}

/// One learning checkpoint over `session`'s unexamined messages: the pending
/// scheduled `/refine` request (if any) always runs first, then the gate.
/// `gate` is `Some` when [`due`] said a review is due (the caller in `rpc.rs`
/// enforces the interval, cooldown and busy checks before calling `pass` at all).
/// Returns a short human summary when something ran.
pub(crate) async fn pass(
    conn: &Arc<Conn>,
    session: &str,
    gate: Option<GateDue>,
) -> anyhow::Result<Option<String>> {
    let complete = conn.config().complete.clone().ok_or_else(|| anyhow::anyhow!("no model available"))?;
    let history = conn.history(session).await?;
    let raw = history["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
    // The model-callable `refine` tool (and the REPL's `refine` host
    // function) never apply mid-turn: they only schedule a request, run here
    // once the turn has actually ended. At most one pending request survives
    // per session (a later call before turn-end just replaces it).
    let store = match factr_learn::entries::EntryStore::open_cached(std::path::Path::new(&conn.config().home)) {
        Ok(store) => Some(store),
        Err(err) => {
            static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("learning is off: cannot open the entry store: {err:#}");
            }
            return Err(anyhow::anyhow!("learning is off: cannot open the entry store ({err:#})"));
        }
    };
    let mut refine_summary = None;
    if let Some(store) = &store {
        if let Ok(Some((instructions, global))) = store.take_pending_refine(session) {
            // An explicit request reads the whole transcript (the refine cap applies), not a signal window.
            let turns = compact_tools(raw.iter());
            let (system, user) = blocking(|| factr_learn::refine::build_request(store, session, &turns, instructions.as_deref(), global));
            let started = crate::observability::now();
            let reply = aux_complete_for(&complete, session, system, user).await;
            conn.observer.record_aux(
                session, "learning", Some("Scheduled refine"), None, None, started,
                reply.as_ref().ok().and_then(|d| d.usage), reply.as_ref().err().map(|e| e.to_string()).as_deref(),
            );
            refine_summary = match reply {
                Ok(done) => match blocking(|| factr_learn::refine::apply_learned(store, session, &done.text, global, "refine-tool", &turns)) {
                    Ok(outcome) => Some(outcome.summary),
                    Err(err) => Some(format!("no change ({err:#})")),
                },
                Err(err) => Some(format!("no change ({err:#})")),
            };
        }
    }
    // Not a checkpoint turn: only the scheduled request above was due.
    let (Some(due), Some(store)) = (gate, &store) else {
        return Ok(refine_summary.map(|s| format!("Refined: {s}.")));
    };
    // The watermark counts raw messages: compaction drops old tool rows, so it can't index `turns`.
    let seen = store.watermark(session).min(raw.len());
    let fresh_owned = window(raw, seen, &due.signals);
    let fresh = &fresh_owned[..];
    if fresh.is_empty() {
        emit(Span::new("learning.skip").session(session).attr("trigger", due.trigger.as_str()).attr("reason", "no_new_messages"));
        return Ok(refine_summary.map(|s| format!("Refined: {s}.")));
    }

    let learned = checkpoint(&complete, store, session, raw.len(), &due, fresh, &mut |title, reply, started| {
        conn.observer.record_aux(
            session, "learning", Some(title), None, None, started,
            reply.as_ref().ok().and_then(|d| d.usage), reply.as_ref().err().map(|e| e.to_string()).as_deref(),
        );
    })
    .await?;
    Ok(match (learned, refine_summary) {
        (Some(summary), Some(refined)) => Some(format!("Learned {summary}; refine: {refined}.")),
        (Some(summary), None) => Some(format!("Learned {summary}.")),
        (None, refined) => refined.map(|s| format!("Refined: {s}.")),
    })
}

/// One auto-refine checkpoint over `fresh` (the messages after the watermark): the gate call,
/// then, when it approves, the refine pass. The watermark and cooldown only move once the window
/// has been judged - a failed model call (429, timeout) leaves both so the next checkpoint
/// re-reviews the same evidence instead of skipping it for good. Returns what was learned.
async fn checkpoint(
    complete: &crate::Complete,
    store: &factr_learn::entries::EntryStore,
    session: &str,
    raw_len: usize,
    due: &GateDue,
    fresh: &[Turn],
    record: &mut (dyn FnMut(&str, &anyhow::Result<factr_provider_core::SimpleCompletion>, i64) + Send),
) -> anyhow::Result<Option<String>> {
    // The gate: one cheap call, asked once a signal is in the window and the cooldown allows it
    // (the caller in `rpc.rs` enforces both).
    let (gsystem, guser) = blocking(|| gate_request(store, session, due, fresh));
    let started = crate::observability::now();
    let greply = aux_complete_for(complete, session, gsystem, guser).await;
    record("Auto-refine gate", &greply, started);
    let gate_span = call_span("learning.gate", session, due.trigger, &greply, started).attr("assistants", due.assistants as u64)
        .attr("signals", kinds(&due.signals))
        .attr("window_chars", window_chars(fresh));
    let review = match &greply {
        Ok(done) => parse_gate_review(&done.text),
        Err(_) => GateReview { should_refine: false, rationale: None, instructions: None },
    };
    emit(gate_span.attr("approved", review.should_refine));
    greply?;
    let judged = || -> anyhow::Result<()> {
        blocking(|| {
            store.set_watermark(session, raw_len)?;
            // The corrections extraction stored are in this window now.
            factr_base::learn_signal::take(session);
            store.learn_reviewed(session, crate::observability::now())
        })
    };
    if !review.should_refine {
        judged()?;
        return Ok(None);
    }
    let outcome = refine_approved(complete, store, session, fresh, due, &review, &mut |reply, started| {
        record("Auto-refine", reply, started)
    })
    .await?;
    judged()?;
    Ok(Some(outcome.map_err(|e| anyhow::anyhow!("{e:#}"))?.summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> crate::Complete {
        std::sync::Arc::new(|_s, _u| Box::pin(async { std::future::pending().await }))
    }

    /// Answers a one-shot call with the model it is on.
    struct Echo(String);

    #[async_trait::async_trait]
    impl factr_base::provider::Provider for Echo {
        async fn complete(
            &self,
            _: &[factr_base::message::Message],
            _: &[factr_base::message::ToolDefinition],
            _: &str,
            _: Option<&str>,
        ) -> anyhow::Result<factr_base::provider::EventStream> {
            Ok(Box::pin(futures_util::stream::empty()))
        }
        fn name(&self) -> &str {
            "echo"
        }
        fn model(&self) -> String {
            self.0.clone()
        }
        fn fork(&self) -> Arc<dyn factr_base::provider::Provider> {
            Arc::new(Echo(self.0.clone()))
        }
        async fn complete_simple_with_usage(&self, _: &str, _: &str) -> anyhow::Result<factr_provider_core::SimpleCompletion> {
            Ok(factr_provider_core::SimpleCompletion { text: self.0.clone(), usage: None })
        }
    }

    #[tokio::test]
    async fn the_learning_calls_run_on_the_chats_model_not_the_startup_one() {
        let startup: Arc<dyn factr_base::provider::Provider> = Arc::new(Echo("startup-model".into()));
        let chat: Arc<dyn factr_base::provider::Provider> = Arc::new(Echo("gpt-5.6-luna".into()));
        factr_base::provider::register_session_provider("learn-model-session", &chat);
        // The shape of the engine's `Complete`: the base is the startup provider unless a session is set.
        let complete: crate::Complete = Arc::new(move |system, user| {
            let provider = factr_base::provider::session_base(startup.clone());
            Box::pin(async move {
                let provider = provider.ok_or_else(|| anyhow::anyhow!("unresolved chat model"))?;
                provider.complete_simple_with_usage(&user, &system).await
            })
        });
        let gate = aux_complete_for(&complete, "learn-model-session", "s".into(), "u".into()).await.unwrap();
        let refine = aux_complete_for(&complete, "learn-model-session", "s".into(), "u".into()).await.unwrap();
        assert_eq!((gate.text.as_str(), refine.text.as_str()), ("gpt-5.6-luna", "gpt-5.6-luna"));
        // An unknown chat (no live agent, nothing saved) fails the call: never the startup model.
        let other = aux_complete_for(&complete, "no-such-session", "s".into(), "u".into()).await;
        assert!(other.is_err(), "no hidden fallback to the startup model");
    }

    #[tokio::test]
    async fn a_busy_slot_or_a_hung_model_fails_the_aux_call_instead_of_queueing_forever() {
        let hung = aux_complete_within(&never(), "s".into(), "u".into(), Duration::from_millis(50)).await.err().expect("fails");
        assert!(hung.to_string().contains("timed out"), "{hung}");
        let held = factr_base::memory_extract::aux_call_permit().await;
        let busy = aux_complete_within(&never(), "s".into(), "u".into(), Duration::from_millis(50)).await.err().expect("fails");
        assert!(busy.to_string().contains("model slot"), "{busy}");
        drop(held);
    }

    #[test]
    fn the_apply_span_error_is_a_class_never_the_text_with_titles() {
        let err = anyhow::anyhow!("rejected: edit for \"Secret client plan\" appears to contain a secret");
        assert_eq!(error_class(&err), "gate_rejected");
        assert_eq!(error_class(&anyhow::anyhow!("no durable lesson in this session")), "no_durable_lesson");
        assert_eq!(error_class(&anyhow::anyhow!("rejected: no_evidence: the quote is not in the conversation")), "no_evidence");
        assert_eq!(error_class(&anyhow::anyhow!("the refine reply was not valid JSON")), "invalid_json");
    }

    #[test]
    fn learning_is_on_by_default_and_config_set_persists_it() {
        let home = std::env::temp_dir().join(format!("learning-setting-{}", std::process::id()));
        let home = home.to_str().unwrap();
        assert!(learning_enabled(home));
        assert!(!set_learning_enabled(home, &serde_json::json!("off")).unwrap());
        assert!(!learning_enabled(home));
        assert!(set_learning_enabled(home, &serde_json::json!(true)).unwrap());
        assert!(learning_enabled(home));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn tool_rows_become_one_bounded_outcome_line_and_only_the_newest_are_kept() {
        use serde_json::json;
        let row = |role: &str, text: String| json!({ "role": role, "content": text });
        let mut rows = vec![row("user", "fix it".into())];
        rows.push(row("tool", format!("\nerror: cannot find x {}\nmore", "y".repeat(300))));
        for i in 0..MAX_TOOL_LINES {
            rows.push(row("tool", format!("done {i}\n{}", "z".repeat(5000))));
        }
        let out = compact_tools(rows.iter());
        let tools: Vec<_> = out.iter().filter(|t| t.role == "tool").collect();
        assert_eq!(tools.len(), MAX_TOOL_LINES, "the oldest (the failure) fell off the cap");
        assert_eq!(tools[0].text, "ok: done 0");
        let one = compact_tools([row("tool", format!("Error: boom {}", "q".repeat(300)))].iter());
        assert!(one[0].text.starts_with("fail: Error: boom") && one[0].text.len() <= 126);
        assert_eq!(out[0].text, "fix it");
        // The row's `tool_data.name` names the tool.
        let named = compact_tools([json!({ "role": "tool", "content": "exit 2\nerror: x", "tool_data": { "name": "grep", "input": {} } })].iter());
        assert_eq!(named[0].text, "ok: grep exit 2");
        let named = compact_tools([json!({ "role": "tool", "content": "Error: nope", "tool_data": { "name": "grep", "input": {} } })].iter());
        assert_eq!(named[0].text, "fail: grep Error: nope");
        // Output that mentions an error is the tool working, not failing.
        let named = compact_tools([json!({ "role": "tool", "content": "error: nope", "tool_data": { "name": "grep", "input": {} } })].iter());
        assert_eq!(named[0].text, "ok: grep error: nope");
    }

    #[test]
    fn watermark_counts_raw_messages_so_compaction_never_skips_unseen_turns() {
        use serde_json::json;
        let mut rows = vec![json!({ "role": "user", "content": "one" })];
        for i in 0..MAX_TOOL_LINES + 10 {
            rows.push(json!({ "role": "tool", "content": format!("done {i}") }));
        }
        let seen = rows.len(); // pass 1 stores the raw count
        rows.push(json!({ "role": "user", "content": "second" }));
        let fresh = compact_tools(rows[seen..].iter());
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].text, "second");
    }

    #[test]
    fn gate_reply_parsing_defaults_to_no() {
        let yes = parse_gate_review("```json\n{\"shouldRefine\": true, \"instructions\": \"save the pnpm rule\"}\n```");
        assert!(yes.should_refine && yes.instructions.as_deref() == Some("save the pnpm rule"));
        assert!(!parse_gate_review("not json at all").should_refine);
        assert!(!parse_gate_review("{\"shouldRefine\": false}").should_refine);
    }

    /// An entry store in its own temp home, removed on drop.
    struct TempStore(factr_learn::entries::EntryStore, std::path::PathBuf);

    impl TempStore {
        fn new() -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let home = std::env::temp_dir().join(format!("learn-store-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            Self(factr_learn::entries::EntryStore::open(&home).unwrap(), home)
        }
    }

    impl std::ops::Deref for TempStore {
        type Target = factr_learn::entries::EntryStore;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.1).ok();
        }
    }

    fn scripted(replies: Vec<&str>) -> (crate::Complete, Arc<std::sync::Mutex<Vec<String>>>) {
        let replies = Arc::new(std::sync::Mutex::new(replies.into_iter().map(String::from).collect::<std::collections::VecDeque<_>>()));
        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = prompts.clone();
        let complete: crate::Complete = Arc::new(move |_system, user| {
            seen.lock().unwrap().push(user);
            let text = replies.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(async move {
                if text == "ERR" {
                    anyhow::bail!("429 rate limited");
                }
                Ok(factr_provider_core::SimpleCompletion { text, usage: None })
            })
        });
        (complete, prompts)
    }

    const LESSON: &str = r#"{"summary":"s","rationale":"r","expectedOutcome":"e","edits":[{"action":"create","kind":"prompt","title":"Nim","content":"Write quick scripts in Nim","evidence":["write quick scripts in Nim"]}]}"#;

    fn gate_due() -> GateDue {
        GateDue { assistants: 25, trigger: Trigger::TurnInterval, signals: vec![Signal { index: 0, kind: SignalKind::UserInstruction }] }
    }

    fn msgs(assistants: usize) -> Vec<Value> {
        let mut rows = vec![serde_json::json!({ "role": "user", "content": "do the thing" })];
        for i in 0..assistants {
            rows.push(serde_json::json!({ "role": "assistant", "content": format!("step {i}") }));
            rows.push(serde_json::json!({ "role": "tool", "content": "ok", "tool_name": "bash" }));
        }
        rows
    }

    fn learning(interval: usize, cooldown: Duration) -> Learning {
        Learning { turn_interval: interval, cooldown }
    }

    fn tool_row(name: &str, input: Value, text: &str) -> Value {
        serde_json::json!({ "role": "tool", "content": text, "tool_data": { "name": name, "input": input } })
    }

    fn user_row(text: &str) -> Value {
        serde_json::json!({ "role": "user", "content": text })
    }

    fn kind_at(found: &[Signal], index: usize) -> Option<SignalKind> {
        found.iter().find(|s| s.index == index).map(|s| s.kind)
    }

    #[test]
    fn thirty_plain_rows_carry_no_signal_and_one_instruction_does() {
        assert!(signals(&msgs(30), 0, false, 0).is_empty(), "work alone is not a cue");
        // "do the thing" is not an instruction; these are.
        for text in ["From now on write quick scripts in Nim", "Remember the staging host is db2", "No, not that file", "Don't touch the lockfile", "I meant the other crate", "use serde instead", "Instead, use serde.", "Never amend a pushed commit", "that's the wrong file"] {
            let mut rows = msgs(5);
            rows.push(user_row(text));
            assert_eq!(kind_at(&signals(&rows, 0, false, 0), rows.len() - 1), Some(SignalKind::UserInstruction), "{text}");
        }
        for text in [
            "now run the tests",
            "I know the answer",
            "a cannon is not a gun",
            "<system-reminder>never stop</system-reminder> continue",
            "stop",
            "no",
            "Add a cache to the parser, and don't change the API.",
            "Let's use a ring buffer instead of a deque for the event queue",
            "Port the CLI to clap. It should never panic on bad flags; print usage.",
        ] {
            let mut rows = msgs(5);
            rows.push(user_row(text));
            assert!(signals(&rows, 0, false, 0).is_empty(), "{text}");
        }
        // A cue opens a session's first request (nothing of the assistant's to correct): a task, not a correction.
        assert!(signals(&[user_row("Don't change the API")], 0, false, 0).is_empty());
        // A reminder-only row between the assistant and the user's reply does not hide the assistant turn.
        let mut rows = msgs(1);
        rows.push(user_row("<system-reminder>context</system-reminder>"));
        rows.push(user_row("Don't change the API"));
        assert_eq!(kind_at(&signals(&rows, 0, false, 0), rows.len() - 1), Some(SignalKind::UserInstruction));
    }

    #[test]
    fn only_the_engines_own_failure_markers_fail_a_tool() {
        for failed in ["Error: no match for old_string", "cargo test\nfailures: 1\n\nExit code: 101"] {
            assert!(tool_failed(failed), "{failed}");
        }
        for ok in [
            "src/log.rs:12: error!(\"disk full\")\nsrc/main.rs:4: // error handling",
            "error: this is the first line of a file the read tool returned",
            "Traceback (most recent call last) appears in this log",
            "the doc says Exit code: 2 means usage",
            "built\n\nExit code: 0",
        ] {
            assert!(!tool_failed(ok), "{ok}");
        }
        // grep output that mentions errors, then the same grep with other arguments: no retry signal.
        let rows = vec![
            tool_row("bash", serde_json::json!({"cmd": "grep -rn error src"}), "src/a.rs:3: error!(\"x\")"),
            tool_row("bash", serde_json::json!({"cmd": "grep -rn warn src"}), "src/a.rs:4: warn!(\"y\")"),
        ];
        assert!(signals(&rows, 0, false, 0).is_empty());
    }

    #[test]
    fn the_loop_guard_and_auto_verify_and_a_retried_tool_are_signals() {
        let mut rows = msgs(2);
        let blocked = rows.len();
        rows.push(tool_row("bash", serde_json::json!({"cmd": "ls"}), "blocked: identical call repeated 6 times, change your approach. bash was not executed."));
        let warned = rows.len();
        rows.push(user_row("<system-reminder>You are repeating yourself: bash has now been called 3 times</system-reminder>"));
        let verify = rows.len();
        rows.push(user_row("<system-reminder>Auto-verify: `cargo test` failed (exit 101, round 1/3). Last output:\nboom</system-reminder>"));
        let found = signals(&rows, 0, false, 0);
        assert_eq!((kind_at(&found, blocked), kind_at(&found, warned), kind_at(&found, verify)), (Some(SignalKind::LoopGuard), Some(SignalKind::LoopGuard), Some(SignalKind::AutoVerify)));
        // Passing auto-verify is not a cue.
        let passed = [user_row("<system-reminder>Auto-verify: `cargo test` passed</system-reminder>")];
        assert!(signals(&passed, 0, false, 0).is_empty());

        // fail then ok with other arguments, within 6 rows
        let mut rows = vec![user_row("fix it")];
        rows.push(tool_row("edit", serde_json::json!({"path": "a"}), "Error: no match"));
        rows.push(serde_json::json!({ "role": "assistant", "content": "trying again" }));
        rows.push(tool_row("edit", serde_json::json!({"path": "b"}), "edited"));
        assert_eq!(kind_at(&signals(&rows, 0, false, 0), 1), Some(SignalKind::ToolRetry));
        // the same arguments again, another tool, or too far away: not a retry
        for second in [tool_row("edit", serde_json::json!({"path": "a"}), "edited"), tool_row("read", serde_json::json!({"path": "b"}), "ok")] {
            let rows = vec![tool_row("edit", serde_json::json!({"path": "a"}), "Error: no match"), second];
            assert!(signals(&rows, 0, false, 0).is_empty());
        }
        let mut far = vec![tool_row("edit", serde_json::json!({"path": "a"}), "Error: no match")];
        far.extend((0..RETRY_ROWS).map(|_| serde_json::json!({ "role": "assistant", "content": "thinking" })));
        far.push(tool_row("edit", serde_json::json!({"path": "b"}), "edited"));
        assert!(signals(&far, 0, false, 0).is_empty());
    }

    #[test]
    fn a_compaction_and_a_stored_correction_point_at_the_newest_row() {
        let rows = msgs(3);
        let found = signals(&rows, 0, true, 2);
        let last = rows.len() - 1;
        assert_eq!((found.len(), kind_at(&found, last)), (2, Some(SignalKind::Compaction)));
        assert_eq!(kinds(&found), "compaction,extraction_correction");
        // only rows since the watermark are read
        let mut rows = vec![user_row("remember the host is db2")];
        rows.extend(msgs(2));
        assert!(signals(&rows, 1, false, 0).is_empty());
    }

    #[test]
    fn the_window_is_the_rows_around_each_signal_capped_at_16k() {
        let mut rows = vec![user_row("start")];
        rows.extend(msgs(20));
        let at = rows.len();
        rows.push(user_row("From now on write quick scripts in Nim"));
        rows.extend((0..10).map(|i| serde_json::json!({ "role": "assistant", "content": format!("after {i}") })));
        let found = signals(&rows, 0, false, 0);
        let texts: Vec<String> = window(&rows, 0, &found).into_iter().map(|t| t.text).collect();
        assert_eq!(texts.len(), WINDOW_BEFORE + 1 + WINDOW_AFTER, "two before, the row, three after: {texts:?}");
        assert_eq!((texts[WINDOW_BEFORE].as_str(), texts.last().unwrap().as_str()), ("From now on write quick scripts in Nim", "after 2"));
        // a window never reaches before the watermark
        assert_eq!(window(&rows, at, &found).len(), 1 + WINDOW_AFTER);
        // two nearby signals union their rows; a huge row is capped, newest first
        let reply = || serde_json::json!({ "role": "assistant", "content": "done" });
        let mut big = vec![reply(), user_row(&"x".repeat(10_000)), reply(), user_row("Don't use unwrap"), user_row(&"y".repeat(10_000)), reply(), user_row("Never use it")];
        big.extend([reply(), user_row("wrong")]);
        let found = signals(&big, 0, false, 0);
        let capped = window(&big, 0, &found);
        assert!(window_chars(&capped) <= SIGNAL_WINDOW_CHARS as u64 && capped.last().unwrap().text == "wrong");
        assert!(window(&rows, 0, &[]).is_empty());
    }

    #[test]
    fn no_signal_is_skipped_and_only_the_ceiling_moves_the_watermark() {
        let store = TempStore::new();
        let l = learning(25, Duration::from_secs(1200));
        let skip = due(&store, "s1", &l, &msgs(24), Trigger::TurnInterval, false, 0, 1_000).unwrap_err();
        assert_eq!((skip.reason, skip.assistants, skip.advanced), ("no_signal", 24, false));
        assert_eq!(store.watermark("s1"), 0);
        let rows = msgs(25);
        let skip = due(&store, "s1", &l, &rows, Trigger::TurnInterval, false, 0, 1_001).unwrap_err();
        assert_eq!((skip.reason, skip.advanced), ("no_signal", true));
        assert_eq!(store.watermark("s1"), rows.len(), "the signal-less stretch is dropped");
    }

    #[test]
    fn a_signal_is_due_unless_cooling_and_only_rows_after_the_watermark_cue() {
        let store = TempStore::new();
        let l = learning(25, Duration::from_millis(1_000));
        let mut rows = msgs(3);
        rows.push(user_row("Remember: the staging host is db2"));
        let d = due(&store, "s1", &l, &rows, Trigger::TurnInterval, false, 0, 5_000).unwrap();
        assert_eq!((d.assistants, d.trigger, kinds(&d.signals).as_str()), (3, Trigger::TurnInterval, "user_instruction"));
        // compaction and extraction are cues of their own; a compaction names the trigger
        let few = msgs(3);
        assert_eq!(due(&store, "s1", &l, &few, Trigger::TurnInterval, true, 0, 5_000).unwrap().trigger, Trigger::Compact);
        assert_eq!(kinds(&due(&store, "s1", &l, &few, Trigger::Dispose, false, 1, 5_000).unwrap().signals), "extraction_correction");
        assert_eq!(due(&store, "s1", &l, &few, Trigger::Dispose, false, 0, 5_000).unwrap_err().reason, "no_signal", "dispose is not a cue");
        // cooling defers a signal, and it is due again after
        store.learn_reviewed("s1", 5_000).unwrap();
        assert_eq!(due(&store, "s1", &l, &rows, Trigger::TurnInterval, false, 0, 5_500).unwrap_err().reason, "cooldown");
        assert!(due(&store, "s1", &l, &rows, Trigger::TurnInterval, false, 0, 6_001).is_ok());
        // the watermark past the instruction row leaves nothing to review
        store.set_watermark("s1", rows.len()).unwrap();
        assert_eq!(due(&store, "s1", &l, &rows, Trigger::TurnInterval, false, 0, 6_002).unwrap_err().reason, "no_signal");
    }

    #[tokio::test]
    async fn an_approved_gate_hands_its_lesson_to_refine_and_a_blank_reply_is_retried_once() {
        let home = std::env::temp_dir().join(format!("learn-handoff-{}", std::process::id()));
        let fresh = vec![Turn { role: "user".into(), text: "From now on write quick scripts in Nim. Remember that.".into() }];
        let review = parse_gate_review(r#"{"shouldRefine": true, "rationale": "stated preference", "instructions": "save: quick scripts in Nim"}"#);
        // First reply: the model says nothing durable; the retry produces the edit.
        let (complete, prompts) = scripted(vec![r#"{"summary":"none","edits":[]}"#, LESSON]);
        let store = factr_learn::entries::EntryStore::open(&home).unwrap();
        let outcome = refine_approved(&complete, &store, "s1", &fresh, &gate_due(), &review, &mut |_, _| {}).await.unwrap().unwrap();
        assert_eq!(outcome.created.len(), 1);
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2, "exactly one retry");
        assert!(prompts[0].contains("save: quick scripts in Nim") && prompts[0].contains("stated preference"), "gate handoff: {}", prompts[0]);
        assert!(prompts[0].contains("write quick scripts in Nim") && !prompts[0].contains("previous reply"));
        assert!(prompts[1].contains("previous reply"), "the retry says why");

        // Two blanks in a row is a genuine miss, surfaced (not looped).
        let (complete, prompts) = scripted(vec!["{\"edits\":[]}", "{\"edits\":[]}", LESSON]);
        let store = factr_learn::entries::EntryStore::open(&home).unwrap();
        let err = refine_approved(&complete, &store, "s1", &fresh, &gate_due(), &review, &mut |_, _| {}).await.unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("no durable lesson") && prompts.lock().unwrap().len() == 2);
        std::fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn an_unquoted_edit_is_retried_once_and_then_rejected() {
        let home = std::env::temp_dir().join(format!("learn-evidence-{}", std::process::id()));
        let store = factr_learn::entries::EntryStore::open(&home).unwrap();
        let fresh = vec![Turn { role: "user".into(), text: "From now on write quick scripts in Nim.".into() }];
        let review = parse_gate_review(r#"{"shouldRefine": true}"#);
        let unquoted = r#"{"summary":"s","edits":[{"action":"create","kind":"prompt","title":"Nim","content":"Write quick scripts in Nim","evidence":["never said this"]}]}"#;
        // The first reply quotes nothing real; the retry (told to quote the user) is stored.
        let (complete, prompts) = scripted(vec![unquoted, LESSON]);
        let done = refine_approved(&complete, &store, "evidence-span", &fresh, &gate_due(), &review, &mut |_, _| {}).await.unwrap().unwrap();
        assert_eq!(done.created.len(), 1);
        assert!(prompts.lock().unwrap()[1].contains("exact words"), "the retry says to quote the user");
        // Two unquoted replies in a row: nothing stored, one retry only.
        let (complete, prompts) = scripted(vec![unquoted, unquoted, LESSON]);
        let home2 = home.join("second");
        let store2 = factr_learn::entries::EntryStore::open(&home2).unwrap();
        assert!(refine_approved(&complete, &store2, "evidence-span-2", &fresh, &gate_due(), &review, &mut |_, _| {}).await.unwrap().is_err());
        assert_eq!(prompts.lock().unwrap().len(), 2);
        assert!(store2.list_visible("evidence-span-2", None).unwrap().is_empty());
        std::fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn a_memory_proposal_is_rejected_and_reported_in_the_apply_span() {
        let home = std::env::temp_dir().join(format!("learn-memory-{}", std::process::id()));
        let store = factr_learn::entries::EntryStore::open(&home).unwrap();
        let fresh = vec![Turn { role: "user".into(), text: "I prefer Nim.".into() }];
        let review = parse_gate_review(r#"{"shouldRefine": true}"#);
        let memory = r#"{"summary":"s","edits":[{"action":"create","kind":"memory","title":"Nim","category":"preference","content":"Prefers Nim"}]}"#;
        let spans = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = spans.clone();
        factr_base::obs_sink::install(move |span| sink.lock().unwrap().push(span));
        let (complete, _) = scripted(vec![memory]);
        let err = refine_approved(&complete, &store, "memory-span", &fresh, &gate_due(), &review, &mut |_, _| {}).await.unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("memory extraction"), "{err:#}");
        assert!(store.list_visible("memory-span", None).unwrap().is_empty());
        let spans = spans.lock().unwrap();
        let apply = spans.iter().find(|s| s.kind == "learning.apply" && s.session_id.as_deref() == Some("memory-span")).expect("apply span");
        assert_eq!(apply.attributes["approved"], false);
        assert_eq!(apply.attributes["error_class"], "gate_rejected");
        assert!(apply.attributes.get("rejected").is_none(), "the span carries a class, not the error text");
        assert_eq!(apply.attributes["created"], 0);
        std::fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn a_failed_gate_call_keeps_the_window_and_cooldown_so_the_next_checkpoint_retries() {
        let home = std::env::temp_dir().join(format!("learn-retry-{}", std::process::id()));
        let store = factr_learn::entries::EntryStore::open(&home).unwrap();
        let fresh = vec![Turn { role: "user".into(), text: "From now on write quick scripts in Nim.".into() }];
        let gate_yes = r#"{"shouldRefine": true, "rationale": "preference", "instructions": "save Nim"}"#;
        let (complete, _) = scripted(vec!["ERR", gate_yes, "ERR", LESSON]);
        let run = |complete: &crate::Complete| {
            let (complete, store, fresh) = (complete.clone(), &store, &fresh);
            async move { checkpoint(&complete, store, "s1", 7, &gate_due(), fresh, &mut |_, _, _| {}).await }
        };
        // Gate 429: nothing is judged, the checkpoint is still due (cooldown untouched).
        assert!(run(&complete).await.is_err());
        assert_eq!(store.watermark("s1"), 0);
        assert!(store.learn_checkpoint("s1", 25, 25, 3_600_000, crate::observability::now(), false).unwrap().is_some());
        // Gate approves but the refine call 429s: still not judged.
        assert!(run(&complete).await.is_err());
        assert_eq!(store.watermark("s1"), 0);
        // Next checkpoint succeeds end to end and only now closes the window and starts the cooldown.
        let (complete, _) = scripted(vec![gate_yes, LESSON]);
        assert!(run(&complete).await.unwrap().is_some());
        assert_eq!(store.watermark("s1"), 7);
        assert!(store.learn_checkpoint("s1", 25, 25, 3_600_000, crate::observability::now(), false).unwrap().is_none());
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn the_gate_sees_the_harness_overview_and_history_and_the_transcript_is_capped_at_40k() {
        let store = TempStore::new();
        factr_learn::refine::apply(&store, "s1", LESSON, false, "refine").unwrap();
        let turns = vec![Turn { role: "user".into(), text: "hello".into() }];
        let (_, user) = gate_request(&store, "s1", &GateDue { assistants: 25, trigger: Trigger::Compact, signals: vec![Signal { index: 0, kind: SignalKind::Compaction }] }, &turns);
        assert!(user.contains("compact; 25 assistant turns") && user.contains("signals: compaction") && user.contains("hello"));
        assert!(user.contains("prompt: 1") && user.contains("Nim") && user.contains("Expected outcome: e"), "{user}");
        let long: Vec<Turn> = (0..3000).map(|i| Turn { role: "user".into(), text: format!("m{i} {}", "x".repeat(50)) }).collect();
        let (_, user) = gate_request(&store, "s1", &gate_due(), &long);
        let conversation = user.split("<conversation>").nth(1).unwrap().split("</conversation>").next().unwrap();
        assert!(conversation.len() <= GATE_TRANSCRIPT_CHARS + 2 && conversation.contains("m2999"));
    }

    #[test]
    fn the_approved_instructions_keep_primes_no_global_promotion_rule() {
        let review = GateReview { should_refine: true, rationale: Some("why".into()), instructions: None };
        let text = approved_instructions(Trigger::Dispose, &review);
        assert!(text.contains("triggered by dispose") && text.contains("Do not promote anything global unless explicitly requested."));
    }
}
