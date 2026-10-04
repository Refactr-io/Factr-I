//! Memory quality gates: pure, code-side checks that decide whether an extracted memory is stored.
//! The model proposes `CATEGORY|CONTENT|MSG_INDEX|QUOTE`; this module decides trust (from where the
//! quote really occurs), rejects junk by rule, weights recall, decays stale memories and audits the
//! store. Nothing here trusts the model.

use crate::memory::{MemoryEntry, MemoryManager, TrustLevel};
use crate::message::{ContentBlock, Message, Role};
use anyhow::Result;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::{LazyLock, OnceLock};

pub const MIN_CHARS: usize = 12;
pub const MAX_CHARS: usize = 400;
/// Learned entries (prompt / skill / subagent) are whole documents, not one-line facts.
pub const MAX_LEARNED_CHARS: usize = 64_000;
pub const MAX_PER_WINDOW: usize = 5;
pub const MIN_QUOTE_CHARS: usize = 8;
const DECAY_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reject {
    /// Old 3-field format or no message index: nothing to verify against.
    Unverifiable,
    BadIndex,
    WeakQuote,
    QuoteNotFound,
    /// The quote only occurs in assistant text: the model's own claim.
    AssistantOnly,
    /// The quote is inside a system-reminder / session-context block, or the text overlaps one.
    SystemContext,
    AbsolutePath,
    Identity,
    Secret,
    CommitHash,
    Ephemeral,
    TooShort,
    TooLong,
    Duplicate,
    OverCap,
}

impl Reject {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unverifiable => "unverifiable",
            Self::BadIndex => "bad_index",
            Self::WeakQuote => "weak_quote",
            Self::QuoteNotFound => "quote_not_found",
            Self::AssistantOnly => "assistant_only",
            Self::SystemContext => "system_context",
            Self::AbsolutePath => "absolute_path",
            Self::Identity => "identity",
            Self::Secret => "secret",
            Self::CommitHash => "commit_hash",
            Self::Ephemeral => "ephemeral",
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
            Self::Duplicate => "duplicate",
            Self::OverCap => "over_cap",
        }
    }
}

impl Reject {
    /// Human reason for an explicit write that was refused ("not stored: ...").
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unverifiable | Self::BadIndex | Self::WeakQuote | Self::QuoteNotFound | Self::AssistantOnly => "it could not be verified",
            Self::SystemContext => "it repeats system or session context",
            Self::AbsolutePath => "contains an absolute path",
            Self::Identity => "names the user or machine identity",
            Self::Secret => "looks like a secret or token",
            Self::CommitHash => "contains a commit hash",
            Self::Ephemeral => "is an ephemeral fact (a version, a current state, a one-off error)",
            Self::TooShort => "is too short (under 12 characters)",
            Self::TooLong => "is too long (over 400 characters)",
            Self::Duplicate => "is a duplicate",
            Self::OverCap => "exceeds the per-window cap",
        }
    }
}

/// Lowercase with collapsed whitespace.
pub fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

const REMINDER_TAGS: [(&str, &str); 2] = [("<system-reminder>", "</system-reminder>"), ("<session-context>", "</session-context>")];

/// Split `text` into (text outside reminder blocks, text inside them). An unclosed block runs to the end.
pub fn split_reminders(text: &str) -> (String, String) {
    let (mut clean, mut inside) = (String::with_capacity(text.len()), String::new());
    let mut rest = text;
    'outer: loop {
        let next = REMINDER_TAGS.iter().filter_map(|(o, c)| rest.find(o).map(|i| (i, *o, *c))).min_by_key(|t| t.0);
        let Some((start, open, close)) = next else { break };
        clean.push_str(&rest[..start]);
        let body = &rest[start + open.len()..];
        match body.find(close) {
            Some(end) => {
                inside.push_str(&body[..end]);
                inside.push('\n');
                rest = &body[end + close.len()..];
            }
            None => {
                inside.push_str(body);
                rest = "";
                break 'outer;
            }
        }
    }
    clean.push_str(rest);
    (clean, inside)
}

/// What one message says, by source, normalized. Tool results are cut at the preview length the
/// model was shown.
#[derive(Default, Debug, Clone)]
pub struct MsgText {
    pub user: String,
    pub tool: String,
    pub assistant: String,
    pub reminder: String,
}

pub const TOOL_PREVIEW_BYTES: usize = 200;

/// What one transcript turn (`role`, `text`) says: the same shape [`msg_text`] gives a message.
/// Any role but `user` and `assistant` is tool output.
pub fn turn_text(role: &str, text: &str) -> MsgText {
    let (clean, inside) = split_reminders(text);
    let mut t = MsgText { reminder: norm(&inside), ..Default::default() };
    let clean = norm(&clean);
    match role {
        "user" => t.user = clean,
        "assistant" => t.assistant = clean,
        _ => t.tool = clean,
    }
    t
}

pub fn msg_text(msg: &Message) -> MsgText {
    let mut t = MsgText::default();
    for block in &msg.content {
        match block {
            ContentBlock::Text { text, .. } => {
                let (clean, inside) = split_reminders(text);
                t.reminder.push_str(&inside);
                t.reminder.push('\n');
                let dest = if msg.role == Role::User { &mut t.user } else { &mut t.assistant };
                dest.push_str(&clean);
                dest.push('\n');
            }
            ContentBlock::ToolResult { content, .. } => {
                t.tool.push_str(crate::util::truncate_str(content, TOOL_PREVIEW_BYTES));
                t.tool.push('\n');
            }
            _ => {}
        }
    }
    for s in [&mut t.user, &mut t.tool, &mut t.assistant, &mut t.reminder] {
        *s = norm(s);
    }
    t
}

/// Where the quote really is decides trust: user text -> High, tool result -> Medium, anything else rejects.
pub fn verify(quote: &str, index: Option<usize>, texts: &[MsgText]) -> Result<TrustLevel, Reject> {
    let index = index.ok_or(Reject::Unverifiable)?;
    let t = texts.get(index).ok_or(Reject::BadIndex)?;
    let q = norm(quote.trim().trim_matches(|c| c == '"' || c == '\''));
    if q.is_empty() {
        return Err(Reject::Unverifiable);
    }
    if q.chars().count() < MIN_QUOTE_CHARS {
        return Err(Reject::WeakQuote);
    }
    if t.user.contains(&q) {
        Ok(TrustLevel::High)
    } else if t.reminder.contains(&q) {
        Err(Reject::SystemContext)
    } else if t.tool.contains(&q) {
        Ok(TrustLevel::Medium)
    } else if t.assistant.contains(&q) {
        Err(Reject::AssistantOnly)
    } else {
        Err(Reject::QuoteNotFound)
    }
}

/// The OS user name and git identity, read once per process.
pub fn identities() -> &'static [String] {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut ids: Vec<String> = ["USER", "USERNAME", "LOGNAME"].iter().filter_map(|k| std::env::var(k).ok()).collect();
        for key in ["user.name", "user.email"] {
            if let Ok(out) = std::process::Command::new("git").args(["config", key]).output()
                && out.status.success()
            {
                ids.push(String::from_utf8_lossy(&out.stdout).trim().to_string());
            }
        }
        ids.retain(|s| s.chars().count() >= 3);
        ids.iter_mut().for_each(|s| *s = s.to_lowercase());
        ids.sort();
        ids.dedup();
        ids
    })
}

pub struct Ctx<'a> {
    pub identities: &'a [String],
    /// Normalized text of every reminder / session-context block in the window.
    pub context: &'a str,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).expect("static regex"));
    };
}
re!(PATH, r#"(?i)((^|[\s"'(=:])/(users|home|root|private|var|tmp|opt|etc|usr|mnt|volumes|srv|workspace|repo)/[\w.\-]+|\b[a-z]:\\|(^|[\s(])~/[\w.\-]+)"#);
re!(SECRET, r"(?i)(\bsk-[a-z0-9_\-]{8,}|\bgh[pousr]_[a-z0-9]{8,}|github_pat_|\bAKIA[0-9A-Z]{12,}|\bxox[abp]-|\bbearer\s+\S{8,}|passw(or)?d)");
re!(LONG_TOKEN, r"[A-Za-z0-9+/_\-]{32,}={0,2}");
re!(HEX, r"(?i)\b[0-9a-f]{7,40}\b");
re!(VERSION, r"\bv?\d+\.\d+(\.\d+)*\b");
re!(
    EPHEMERAL,
    r"(?i)(not installed|isn'?t installed|\bno \w+ installed|\bcurrently\b|right now|at the moment|at present|\btoday\b|yesterday|tonight|this session|for now|temporarily|command not found|system python|not available|unavailable|\bis missing\b|\bwas missing\b|\bpip install|\btraceback\b|exit code|timed out|\bhas no \w+ (module|package|installed))"
);

fn has_secret(text: &str) -> bool {
    SECRET.is_match(text)
        || LONG_TOKEN.find_iter(text).any(|m| m.as_str().chars().any(|c| c.is_ascii_digit()) && !m.as_str().contains(char::is_whitespace))
}

fn has_commit_hash(text: &str) -> bool {
    HEX.find_iter(text).any(|m| {
        let s = m.as_str();
        s.chars().any(|c| c.is_ascii_digit()) && s.chars().any(|c| c.is_ascii_alphabetic())
    })
}

fn names_identity(text: &str, ids: &[String]) -> bool {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric())).collect();
    ids.iter().any(|id| {
        if id.contains(char::is_whitespace) {
            lower.contains(id.as_str())
        } else {
            tokens.contains(&id.as_str())
        }
    })
}

/// A run of six words of `text` that also occurs verbatim in `context`.
fn overlaps(text: &str, context: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    words.len() >= 6 && words.windows(6).any(|w| context.contains(&w.join(" ")))
}

/// Every content rule. `trust` is what the code decided (or the stored trust, for an audit).
pub fn check(content: &str, quote: Option<&str>, trust: TrustLevel, ctx: &Ctx) -> Result<(), Reject> {
    let n = content.trim().chars().count();
    if n < MIN_CHARS {
        return Err(Reject::TooShort);
    }
    if n > MAX_CHARS {
        return Err(Reject::TooLong);
    }
    if PATH.is_match(content) {
        return Err(Reject::AbsolutePath);
    }
    if has_secret(content) {
        return Err(Reject::Secret);
    }
    // The user stating their own name is legitimate; the same name copied from tool output or repo
    // context is not.
    if trust != TrustLevel::High && names_identity(content, ctx.identities) {
        return Err(Reject::Identity);
    }
    if has_commit_hash(content) {
        return Err(Reject::CommitHash);
    }
    if EPHEMERAL.is_match(content) || (trust != TrustLevel::High && VERSION.is_match(content)) {
        return Err(Reject::Ephemeral);
    }
    if !ctx.context.is_empty() {
        let c = norm(content);
        if overlaps(&c, ctx.context) || quote.is_some_and(|q| ctx.context.contains(&norm(q))) && trust != TrustLevel::High {
            return Err(Reject::SystemContext);
        }
    }
    Ok(())
}

/// The one gate for explicit writes (the `memory` tool, desktop REST add/edit, upserts): the code-side
/// content filters, no provenance quote (the writer is the model or the user, trusted by source).
/// Learned prompt/skill/subagent entries skip the fact rules (a skill may name paths and versions by
/// design) and go through [`gate_learned`] instead. Exact repeats are not rejected here: the store
/// reinforces them.
pub fn gate_explicit(content: &str, category_is_learned: bool, trust: TrustLevel) -> Result<()> {
    if category_is_learned {
        return Ok(());
    }
    let ctx = Ctx { identities: identities(), context: "" };
    check(content, None, trust, &ctx).map_err(|reason| anyhow::anyhow!("not stored: {}", reason.reason()))
}

/// `gate_explicit` for a whole entry: its own category and trust decide.
pub fn gate_entry(entry: &MemoryEntry) -> Result<()> {
    if entry.category.is_learned() {
        return gate_learned(entry);
    }
    gate_explicit(&entry.content, entry.category.is_learned(), entry.trust)
}

/// The gate for a learned entry: not empty, not a runaway document. Duplicates are not a rule: a
/// learned entry is keyed by id and replaced by it (never merged by similarity), so a rewrite of
/// the same id is an update, and factr-learn owns what its learner chooses to keep.
pub fn gate_learned(entry: &MemoryEntry) -> Result<()> {
    let n = entry.content.trim().chars().count();
    let titled = entry.learned.as_ref().is_some_and(|l| !l.title.trim().is_empty());
    if n == 0 && !titled {
        return Err(anyhow::anyhow!("not stored: empty learned entry"));
    }
    if n > MAX_LEARNED_CHARS {
        return Err(anyhow::anyhow!("not stored: learned entry over {MAX_LEARNED_CHARS} characters"));
    }
    Ok(())
}

/// Weight of an entry for recall: trust, reinforcement and recency. Never below 0.35, so low-trust
/// and old memories are ranked down, not hidden.
pub fn recall_weight(e: &MemoryEntry, now: chrono::DateTime<chrono::Utc>) -> f32 {
    let trust = match e.trust {
        TrustLevel::High => 1.0,
        TrustLevel::Medium => 0.85,
        TrustLevel::Low => 0.7,
    };
    let reinforced = 1.0 + 0.1 * e.strength.saturating_sub(1).min(5) as f32;
    let days = (now - e.updated_at).num_days().max(0) as f32;
    let recency = 1.0 - 0.4 * (days / 365.0).min(1.0);
    (trust * reinforced * recency).max(0.35)
}

/// Re-rank BM25-ordered candidates by `recall_weight` (stable: ties keep BM25 order).
pub fn rerank(entries: Vec<MemoryEntry>, now: chrono::DateTime<chrono::Utc>) -> Vec<MemoryEntry> {
    let n = entries.len().max(1) as f32;
    let mut scored: Vec<(f32, MemoryEntry)> =
        entries.into_iter().enumerate().map(|(i, e)| ((1.0 - 0.5 * i as f32 / n) * recall_weight(&e, now), e)).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, e)| e).collect()
}

#[derive(Default, Debug)]
pub struct AuditReport {
    pub scanned: usize,
    pub counts: BTreeMap<&'static str, usize>,
    /// (id, content) of every rejected memory.
    pub rejected: Vec<(String, String)>,
    pub applied: usize,
    /// (survivor id, duplicate id, duplicate content) of every active memory that repeats another.
    pub duplicates: Vec<(String, String, String)>,
    /// Duplicates folded into their survivor (with `apply`).
    pub merged: usize,
}

impl MemoryManager {
    /// Mark stale, unproven memories inactive: trust low or medium, never reinforced, untouched for
    /// 30 days. High-trust and learned entries are never expired. "Touched" is `updated_at` (set on
    /// write and on reinforcement); recall does not record access.
    pub fn decay_stale(&self, now: chrono::DateTime<chrono::Utc>) -> Result<usize> {
        let mut stale = Vec::new();
        for (_, graph) in self.every_scope_graph()? {
            for e in graph.active_memories() {
                if !e.category.is_learned()
                    && e.trust != TrustLevel::High
                    && e.strength <= 1
                    && (now - e.updated_at).num_days() >= DECAY_DAYS
                {
                    stale.push(e.id.clone());
                }
            }
        }
        for id in &stale {
            self.expire(id, "decay: unreinforced for 30 days")?;
        }
        Ok(stale.len())
    }

    /// Run the content filters over every active, non-learned, non-high-trust memory. With `apply`,
    /// mark the rejects inactive with reason `audit`; rows are never deleted.
    pub fn audit(&self, apply: bool) -> Result<AuditReport> {
        let mut report = AuditReport::default();
        let ctx = Ctx { identities: identities(), context: "" };
        let mut kept: Vec<(String, MemoryEntry)> = Vec::new();
        for (scope, graph) in self.every_scope_graph()? {
            for e in graph.active_memories() {
                if e.category.is_learned() {
                    continue;
                }
                if e.trust != TrustLevel::High {
                    report.scanned += 1;
                    if let Err(reason) = check(&e.content, None, e.trust, &ctx) {
                        *report.counts.entry(reason.as_str()).or_default() += 1;
                        report.rejected.push((e.id.clone(), e.content.clone()));
                        continue;
                    }
                }
                kept.push((scope.clone(), e.clone()));
            }
        }
        // Duplicates: global rows first, then higher trust and strength, then older, survive.
        kept.sort_by(|(sa, a), (sb, b)| {
            ((sb == "global").cmp(&(sa == "global")))
                .then(crate::memory_store::trust_order(&b.trust).cmp(&crate::memory_store::trust_order(&a.trust)))
                .then(b.strength.cmp(&a.strength))
                .then(a.created_at.cmp(&b.created_at))
        });
        let mut folded = vec![false; kept.len()];
        let mut pairs: Vec<((String, String), (String, String))> = Vec::new();
        for i in 0..kept.len() {
            if folded[i] {
                continue;
            }
            for j in i + 1..kept.len() {
                if !folded[j]
                    && kept[i].1.category == kept[j].1.category
                    && crate::memory_store::same_fact(&kept[i].1.category, &kept[i].1.content, &kept[j].1.content).is_some()
                    // Project facts stay per project; only preferences are about the user.
                    && (kept[i].0 == kept[j].0 || kept[i].1.category == crate::memory_types::MemoryCategory::Preference || kept[i].0 == "global")
                {
                    folded[j] = true;
                    report.duplicates.push((kept[i].1.id.clone(), kept[j].1.id.clone(), kept[j].1.content.clone()));
                    pairs.push(((kept[i].0.clone(), kept[i].1.id.clone()), (kept[j].0.clone(), kept[j].1.id.clone())));
                }
            }
        }
        if apply {
            for (id, _) in &report.rejected {
                if self.expire(id, "audit")? {
                    report.applied += 1;
                }
            }
            for (survivor, duplicate) in &pairs {
                if self.merge_duplicate((&survivor.0, &survivor.1), (&duplicate.0, &duplicate.1))? {
                    report.merged += 1;
                }
            }
        }
        Ok(report)
    }
}

/// The `factr memory audit [--apply]` report text.
pub fn run_audit(apply: bool) -> Result<String> {
    let report = MemoryManager::new().audit(apply)?;
    let mut out = format!("scanned {} memories, {} rejected{}\n", report.scanned, report.rejected.len(), if apply { "" } else { " (dry run, pass --apply to mark inactive)" });
    for (reason, n) in &report.counts {
        out.push_str(&format!("  {reason}: {n}\n"));
    }
    for (id, content) in report.rejected.iter().take(20) {
        out.push_str(&format!("  {id}  {}\n", content.chars().take(60).collect::<String>().replace('\n', " ")));
    }
    out.push_str(&format!("duplicates: {}{}\n", report.duplicates.len(), if apply { "" } else { " (dry run, pass --apply to merge)" }));
    for (survivor, id, content) in report.duplicates.iter().take(20) {
        out.push_str(&format!("  {id} -> {survivor}  {}\n", content.chars().take(60).collect::<String>().replace('\n', " ")));
    }
    if apply {
        out.push_str(&format!("marked inactive: {}\nmerged duplicates: {}\n", report.applied, report.merged));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "memory_quality_tests.rs"]
mod tests;
