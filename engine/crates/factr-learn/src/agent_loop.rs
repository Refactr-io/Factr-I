//! Session goals and the per-session scheduler (factr-learn long-running agents semantics, Factr
//! `session.control` wire shape).
//!
//! `/goal` is the one unattended-work command: budgets, quality gates, a completion contract, a
//! wait barrier. `/loop` (aliases `/heartbeat`, `/hb`, `/proactive`) is the one scheduler: a
//! recurring prompt with optional `--times` and `--until`, stored as a heartbeat.
//!
//! Persisted in `factr.db` so state survives engine restart and desktop
//! disconnect/reattach. The desktop reads `session.control.read`; a scheduler with `--times` or
//! `--until` is projected into the `loop` field, a plain one into `heartbeat`.

use crate::goal_ratchet::{Checkpoint, Score, command_key, score_from_json, score_of, score_to_json};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SCHEMA: &str = "
    PRAGMA journal_mode=WAL;
    PRAGMA busy_timeout=5000;
    CREATE TABLE IF NOT EXISTS session_goals(
        session_id TEXT PRIMARY KEY,
        state TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS session_heartbeats(
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        source TEXT NOT NULL DEFAULT 'user',
        state TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS session_heartbeats_session ON session_heartbeats(session_id, updated_at_ms);
";

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_secs_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Empty completion-contract fields the desktop parser requires.
fn empty_contract() -> Value {
    json!({
        "outcome": "",
        "verification": "",
        "constraints": "",
        "boundaries": "",
        "stop_when": ""
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualityGate {
    pub command: String,
    pub timeout_seconds: i64,
    pub max_retries: i64,
    pub attempts: i64,
    pub last_exit_code: Option<i64>,
}

impl QualityGate {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            timeout_seconds: 120,
            max_retries: 3,
            attempts: 0,
            last_exit_code: None,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "command": self.command,
            "timeout_seconds": self.timeout_seconds,
            "max_retries": self.max_retries,
            "attempts": self.attempts,
            "last_exit_code": self.last_exit_code,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            command: v["command"].as_str()?.to_string(),
            timeout_seconds: v["timeout_seconds"].as_i64().unwrap_or(120),
            max_retries: v["max_retries"].as_i64().unwrap_or(3),
            attempts: v["attempts"].as_i64().unwrap_or(0),
            last_exit_code: v["last_exit_code"].as_i64(),
        })
    }
}

/// Run a quality gate in `cwd`. A passed gate means only that gate passed.
pub fn run_gate(gate: &mut QualityGate, cwd: Option<&Path>) -> GateResult {
    gate.attempts += 1;
    let r = run_command(&gate.command, cwd, Duration::from_secs(gate.timeout_seconds.max(1) as u64));
    gate.last_exit_code = Some(r.exit_code);
    r
}

/// The shell invocation for a gate or one-shot command: `bash -c` (else `/bin/sh -c`) on Unix, `cmd.exe /D /S /C` on Windows
/// (`/D` skips AutoRun hooks, `/S` plus the outer quotes keep the inner quotes intact for child programs).
fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/D", "/S", "/C"]).raw_arg(format!("\"{command}\""));
        cmd
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new(factr_base::shell::posix_shell());
        cmd.arg("-c").arg(command).process_group(0);
        // The gate commands see the session venv like `bash` does: wait for the build (free when there
        // is none or it is over), and drop the venv variables when it failed.
        factr_base::python_env::ensure_session_venv(factr_base::python_env::VENV_WAIT);
        if let Some(path) = factr_base::python_env::withdrawn_path() {
            cmd.env("PATH", path).env_remove("FACTR_SESSION_VENV").env_remove("VIRTUAL_ENV");
        }
        cmd
    }
}

/// Run `command` through the platform shell in `cwd` in its own process group (Unix) or console process
/// group with its tree killed by `taskkill /T` (Windows). Pipes are drained on
/// separate threads (no buffer deadlock); on timeout the whole group is killed.
/// `output` is the last 3000 chars of stdout+stderr (exit 124 = timeout).
pub fn run_command(command: &str, cwd: Option<&Path>, timeout: Duration) -> GateResult {
    use std::io::Read;
    let mut cmd = shell_command(command);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        cmd.creation_flags(0x0000_0200 | 0x0800_0000);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            return GateResult { passed: false, exit_code: 127, output: format!("failed to spawn gate: {err}") };
        }
    };
    // Keep only the tail of each stream while draining.
    fn drain<R: Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut tail = Vec::new();
            let Some(mut r) = r else { return tail };
            let mut buf = [0u8; 8192];
            while let Ok(n) = r.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > 64 * 1024 {
                    let cut = tail.len() - 32 * 1024;
                    tail.drain(..cut);
                }
            }
            tail
        })
    }
    let out_h = drain(child.stdout.take());
    let err_h = drain(child.stderr.take());
    let pgid = child.id();
    let started = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code().unwrap_or(1) as i64, false),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = factr_base::platform::signal_detached_process_group(pgid, 9);
                let _ = child.kill();
                let _ = child.wait();
                break (124, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break (1, false),
        }
    };
    // Grandchildren holding the pipes open would block the join; reap the group.
    // (Windows: the leader is gone by now and its pid may be reused, so only a timeout kills the tree.)
    #[cfg(unix)]
    let _ = factr_base::platform::signal_detached_process_group(pgid, 9);
    let mut text = String::from_utf8_lossy(&err_h.join().unwrap_or_default()).into_owned();
    // stderr first keeps test failures (usually stderr) nearest the tail.
    text.insert_str(0, &String::from_utf8_lossy(&out_h.join().unwrap_or_default()));
    if timed_out {
        text.push_str(&format!("\ngate timed out after {}s", timeout.as_secs()));
    }
    let output = text.chars().rev().take(3000).collect::<String>().chars().rev().collect();
    GateResult { passed: code == 0, exit_code: code, output }
}

#[derive(Debug, Clone)]
pub struct GateResult {
    pub passed: bool,
    pub exit_code: i64,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalStatus {
    Active,
    Done,
    Paused,
}

impl GoalStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Done => "done",
            Self::Paused => "paused",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "done" => Some(Self::Done),
            "paused" => Some(Self::Paused),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionGoal {
    pub title: String,
    pub status: GoalStatus,
    pub turns_used: i64,
    pub max_turns: i64,
    pub token_budget: Option<i64>,
    pub tokens_used: i64,
    pub wall_budget_ms: Option<i64>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    pub created_at_ms: i64,
    pub paused_reason: Option<String>,
    pub last_verdict: Option<String>,
    pub last_reason: Option<String>,
    pub subgoals: Vec<String>,
    pub gates: Vec<QualityGate>,
    pub contract: Value,
    /// Pause continuations while waiting on subagents.
    pub waiting_on_subagents: bool,
    /// Capped, one-line-per-turn attempt log: what was tried, whether
    /// verification passed, and the key error if it failed. Only ever
    /// injected into the per-turn continuation prompt (never the cached
    /// static prefix), and capped at `MAX_ATTEMPT_LOG` entries of at most
    /// `MAX_ATTEMPT_LEN` chars each so it stays a small, bounded addition.
    pub attempt_log: Vec<String>,
    /// The verification the model cited when it called `complete` (what it
    /// ran and the result). Required for completion; also surfaced in the
    /// budget-exhaustion report so a paused/expired goal still shows what
    /// was verified so far.
    pub completion_verification: Option<String>,
    /// AVO ratchet (see `goal_ratchet`): best score per test command,
    /// checkpoint lineage (cap 20), regression note, supervisor rate limits.
    pub best: Score,
    pub lineage: Vec<Checkpoint>,
    pub checkpoint_seq: u32,
    pub regressed: Option<String>,
    pub sup_turn: Option<i64>,
    pub sup_episode: bool,
    /// Repo the checkpoint refs were written in (so clear/complete can delete them).
    pub ref_cwd: Option<String>,
    /// Verification commands run for this goal (pass or fail), whatever `best` holds, and whether
    /// the latest verifying turn passed them all.
    pub verify_runs: u32,
    pub verify_ok: bool,
    /// `/goal wait <pid> [reason]`: parked until that process exits (or `/goal unwait`).
    pub wait_pid: Option<i64>,
    pub wait_reason: Option<String>,
    /// The model asked to complete while quality gates exist: the verification it cited, held until
    /// the gates have run at the end of the turn (see `after_turn_in`).
    pub completion_pending: Option<String>,
}

/// Attempt-log bounds (NVIDIA AVO long-horizon harness: a small, bounded
/// per-turn note, not unbounded history).
pub const MAX_ATTEMPT_LOG: usize = 8;
pub const MAX_ATTEMPT_LEN: usize = 80;
/// Consecutive auto-recorded turns examined for plateau detection.
pub const PLATEAU_TURNS: usize = 3;

/// What the engine observed in one turn, accumulated from `tool.complete`
/// events (no model call, no prompt tokens).
#[derive(Default)]
struct TurnObs {
    tools: u32,
    failed: u32,
    files_changed: bool,
    verified: bool,
    first_err: Option<String>,
    scores: Score,
}

fn turn_obs() -> &'static Mutex<HashMap<String, TurnObs>> {
    static OBS: OnceLock<Mutex<HashMap<String, TurnObs>>> = OnceLock::new();
    OBS.get_or_init(Default::default)
}

/// Strip paths and digits so identical failures compare equal.
fn normalize_error(err: &str) -> String {
    let first = err.trim().trim_start_matches("Error:").trim().lines().next().unwrap_or("");
    let words: Vec<String> = first
        .split_whitespace()
        .map(|w| {
            if w.contains('/') || w.contains('\\') {
                "<p>".to_string()
            } else {
                w.chars().filter(|c| !c.is_ascii_digit()).collect()
            }
        })
        .collect();
    words.join(" ").to_lowercase().chars().take(30).collect()
}

/// Record one completed tool call for `session_id`'s current turn. Called by
/// the gateway at `tool.complete`; drained by `after_turn`.
pub fn observe_tool(session_id: &str, name: &str, args: &Value, result: &str) {
    // factr's bash tool reports a non-zero exit as a trailing "Exit code: N"
    // (or "finished with exit code: N" for detached runs), not an "Error:" prefix.
    let nonzero_exit = result.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| {
        let l = l.trim().trim_end_matches(" ---");
        l.rsplit_once("xit code: ").is_some_and(|(_, code)| code.trim() != "0")
    });
    let failed = result.starts_with("Error:") || nonzero_exit;
    let cmd = args["command"].as_str().unwrap_or("").to_lowercase();
    let verify_cmd = name == "bash" && crate::goal_ratchet::is_verify_command(&cmd);
    let mut map = turn_obs().lock().unwrap_or_else(|e| e.into_inner());
    let obs = map.entry(session_id.to_string()).or_default();
    obs.tools += 1;
    if verify_cmd {
        obs.scores.insert(command_key(&cmd), score_of(!failed, result));
    }
    if failed {
        obs.failed += 1;
        obs.first_err.get_or_insert_with(|| normalize_error(result));
        return;
    }
    if matches!(name, "edit" | "write" | "patch" | "apply_patch" | "multiedit") {
        obs.files_changed = true;
    }
    if verify_cmd {
        obs.verified = true;
    }
}

impl SessionGoal {
    pub fn new(title: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            title: title.into(),
            status: GoalStatus::Active,
            turns_used: 0,
            max_turns: 20,
            token_budget: None,
            tokens_used: 0,
            wall_budget_ms: None,
            started_at_ms: now,
            updated_at_ms: now,
            created_at_ms: now,
            paused_reason: None,
            last_verdict: None,
            last_reason: None,
            subgoals: Vec::new(),
            gates: Vec::new(),
            contract: empty_contract(),
            waiting_on_subagents: false,
            attempt_log: Vec::new(),
            completion_verification: None,
            best: Score::new(),
            lineage: Vec::new(),
            checkpoint_seq: 0,
            regressed: None,
            sup_turn: None,
            sup_episode: false,
            ref_cwd: None,
            verify_runs: 0,
            verify_ok: false,
            wait_pid: None,
            wait_reason: None,
            completion_pending: None,
        }
    }

    /// Record one capped, one-line attempt-log entry for this turn. Never
    /// ends the goal on a failed tool call or failed verification — only
    /// completion, budget exhaustion, or a user cancel do that.
    pub fn record_attempt(&mut self, line: impl Into<String>) {
        let mut line = line.into();
        if line.chars().count() > MAX_ATTEMPT_LEN {
            line = line
                .chars()
                .take(MAX_ATTEMPT_LEN.saturating_sub(1))
                .collect::<String>();
            line.push('…');
        }
        self.attempt_log.push(line);
        if self.attempt_log.len() > MAX_ATTEMPT_LOG {
            let excess = self.attempt_log.len() - MAX_ATTEMPT_LOG;
            self.attempt_log.drain(0..excess);
        }
    }

    /// Cheap, no-model-call plateau heuristic. Over the last
    /// `PLATEAU_TURNS` engine-recorded `auto` lines: no successful
    /// verification AND (same normalized error signature OR no file
    /// changes). Also trips when the last 3 entries of any kind are
    /// identical (model-supplied notes that repeat).
    pub fn plateaued(&self) -> bool {
        let n = self.attempt_log.len();
        if n >= PLATEAU_TURNS {
            let last = &self.attempt_log[n - PLATEAU_TURNS..];
            if last.iter().all(|line| line == &last[0] && !line.starts_with("auto ")) {
                return true;
            }
        }
        let auto: Vec<&str> = self
            .attempt_log
            .iter()
            .filter(|l| l.starts_with("auto "))
            .map(String::as_str)
            .collect();
        if auto.len() < PLATEAU_TURNS {
            return false;
        }
        let last = &auto[auto.len() - PLATEAU_TURNS..];
        let field = |l: &str, key: &str| -> String {
            l.split(" | ")
                .find_map(|p| p.strip_prefix(key))
                .unwrap_or("")
                .to_string()
        };
        if last.iter().any(|l| field(l, "ver=") == "pass") {
            return false;
        }
        let err0 = field(last[0], "err=");
        let same_err = !err0.is_empty() && last.iter().all(|l| field(l, "err=") == err0);
        let no_files = last.iter().all(|l| field(l, "files=") == "no");
        same_err || no_files
    }

    pub fn out_of_budget(&self) -> Option<&'static str> {
        if self.max_turns > 0 && self.turns_used >= self.max_turns {
            return Some("turn budget");
        }
        if let Some(tokens) = self.token_budget {
            if tokens > 0 && self.tokens_used >= tokens {
                return Some("token budget");
            }
        }
        if let Some(wall) = self.wall_budget_ms {
            if wall > 0 && now_ms().saturating_sub(self.started_at_ms) >= wall {
                return Some("wall-clock budget");
            }
        }
        None
    }

    /// The desktop's goal chip: exactly the fields its parser allows (`parseGoal` in the desktop's
    /// session-control store rejects the whole snapshot on any other key).
    pub fn to_control_json(&self) -> Value {
        let mut v = json!({
            "title": self.title,
            "status": self.status.as_str(),
            "turns_used": self.turns_used,
            "max_turns": self.max_turns,
            "contract": self.contract,
            "subgoals": self.subgoals,
            "gates": self.gates.iter().map(QualityGate::to_json).collect::<Vec<_>>(),
            "created_at": self.created_at_ms as f64 / 1000.0,
            "updated_at": self.updated_at_ms as f64 / 1000.0,
        });
        if let Some(reason) = &self.paused_reason {
            v["paused_reason"] = json!(reason);
        }
        if let Some(verdict) = &self.last_verdict {
            v["last_verdict"] = json!(verdict);
        }
        if let Some(reason) = &self.last_reason {
            v["last_reason"] = json!(reason);
        }
        v
    }

    fn to_json(&self) -> Value {
        json!({
            "title": self.title,
            "status": self.status.as_str(),
            "turns_used": self.turns_used,
            "max_turns": self.max_turns,
            "token_budget": self.token_budget,
            "tokens_used": self.tokens_used,
            "wall_budget_ms": self.wall_budget_ms,
            "started_at_ms": self.started_at_ms,
            "updated_at_ms": self.updated_at_ms,
            "created_at_ms": self.created_at_ms,
            "paused_reason": self.paused_reason,
            "last_verdict": self.last_verdict,
            "last_reason": self.last_reason,
            "subgoals": self.subgoals,
            "gates": self.gates.iter().map(QualityGate::to_json).collect::<Vec<_>>(),
            "contract": self.contract,
            "waiting_on_subagents": self.waiting_on_subagents,
            "attempt_log": self.attempt_log,
            "completion_verification": self.completion_verification,
            "best": score_to_json(&self.best),
            "lineage": self.lineage.iter().map(Checkpoint::to_json).collect::<Vec<_>>(),
            "checkpoint_seq": self.checkpoint_seq,
            "regressed": self.regressed,
            "sup_turn": self.sup_turn,
            "sup_episode": self.sup_episode,
            "ref_cwd": self.ref_cwd,
            "verify_runs": self.verify_runs,
            "verify_ok": self.verify_ok,
            "wait_pid": self.wait_pid,
            "wait_reason": self.wait_reason,
            "completion_pending": self.completion_pending,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            title: v["title"].as_str()?.to_string(),
            status: GoalStatus::parse(v["status"].as_str()?)?,
            turns_used: v["turns_used"].as_i64().unwrap_or(0),
            max_turns: v["max_turns"].as_i64().unwrap_or(20),
            token_budget: v["token_budget"].as_i64(),
            tokens_used: v["tokens_used"].as_i64().unwrap_or(0),
            wall_budget_ms: v["wall_budget_ms"].as_i64(),
            started_at_ms: v["started_at_ms"].as_i64().unwrap_or(0),
            updated_at_ms: v["updated_at_ms"].as_i64().unwrap_or(0),
            created_at_ms: v["created_at_ms"].as_i64().unwrap_or(0),
            paused_reason: v["paused_reason"].as_str().map(str::to_string),
            last_verdict: v["last_verdict"].as_str().map(str::to_string),
            last_reason: v["last_reason"].as_str().map(str::to_string),
            subgoals: v["subgoals"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            gates: v["gates"]
                .as_array()
                .map(|a| a.iter().filter_map(QualityGate::from_json).collect())
                .unwrap_or_default(),
            contract: if v["contract"].is_object() {
                v["contract"].clone()
            } else {
                empty_contract()
            },
            waiting_on_subagents: v["waiting_on_subagents"].as_bool().unwrap_or(false),
            attempt_log: v["attempt_log"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            completion_verification: v["completion_verification"]
                .as_str()
                .map(str::to_string),
            best: score_from_json(&v["best"]),
            lineage: v["lineage"].as_array().map(|a| a.iter().filter_map(Checkpoint::from_json).collect()).unwrap_or_default(),
            checkpoint_seq: v["checkpoint_seq"].as_u64().unwrap_or(0) as u32,
            regressed: v["regressed"].as_str().map(str::to_string),
            sup_turn: v["sup_turn"].as_i64(),
            sup_episode: v["sup_episode"].as_bool().unwrap_or(false),
            ref_cwd: v["ref_cwd"].as_str().map(str::to_string),
            verify_runs: v["verify_runs"].as_u64().unwrap_or(0) as u32,
            verify_ok: v["verify_ok"].as_bool().unwrap_or(false),
            wait_pid: v["wait_pid"].as_i64(),
            wait_reason: v["wait_reason"].as_str().map(str::to_string),
            completion_pending: v["completion_pending"].as_str().map(str::to_string),
        })
    }

    pub fn status_text(&self) -> String {
        match self.status {
            GoalStatus::Active => {
                if let Some(pid) = self.wait_pid {
                    let why = self.wait_reason.as_deref().filter(|r| !r.is_empty()).map(|r| format!(", {r}")).unwrap_or_default();
                    format!("⏳ Goal (parked on pid {pid}{why}, {}/{} turns): {}", self.turns_used, self.max_turns, self.title)
                } else if self.waiting_on_subagents {
                    format!(
                        "⏳ Goal (parked, {}/{} turns): {}",
                        self.turns_used, self.max_turns, self.title
                    )
                } else {
                    format!(
                        "⊙ Goal (active, {}/{} turns): {}",
                        self.turns_used, self.max_turns, self.title
                    )
                }
            }
            GoalStatus::Paused => {
                let detail = self.paused_reason.as_deref().unwrap_or("paused");
                format!(
                    "⏸ Goal paused — {}. Use /goal resume to keep going.",
                    detail
                )
            }
            GoalStatus::Done => format!(
                "✓ Goal done ({}/{} turns): {}",
                self.turns_used, self.max_turns, self.title
            ),
        }
    }

    pub fn continuation_prompt(&self) -> String {
        self.continuation_prompt_with(None)
    }

    /// The completion contract's non-empty fields (`outcome`, `verification`, ...) as a labelled
    /// block, then the extra criteria; empty for a bare free-form goal.
    pub fn contract_block(&self) -> String {
        let labels = [("outcome", "Outcome"), ("verification", "Verification"), ("constraints", "Constraints"), ("boundaries", "Boundaries"), ("stop_when", "Stop when blocked")];
        let mut lines: Vec<String> = labels
            .iter()
            .filter_map(|(key, label)| self.contract[*key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(|v| format!("- {label}: {v}")))
            .collect();
        lines.extend(self.subgoals.iter().enumerate().map(|(i, text)| format!("- Extra criterion {}: {text}", i + 1)));
        lines.join("\n")
    }

    /// `supervisor`: alternative strategies from the one rare supervisor call,
    /// injected once into this prompt (never the cached static prefix).
    pub fn continuation_prompt_with(&self, supervisor: Option<&str>) -> String {
        let mut log_block = String::new();
        if !self.attempt_log.is_empty() {
            log_block.push_str("\n\nRecent attempts (most recent last):\n");
            for line in &self.attempt_log {
                log_block.push_str("- ");
                log_block.push_str(line);
                log_block.push('\n');
            }
        }
        log_block.push_str(&self.ratchet_block());
        let steer = if let Some(alt) = supervisor {
            format!("\n[Supervisor: alternative directions]\n{}", alt.chars().take(600).collect::<String>())
        } else if self.plateaued() {
            "\n[Plateau detected] The last few turns made no new verified progress \
             (same result repeated). Abandon the current approach and try a genuinely \
             different strategy instead of repeating what already failed above.".to_string()
        } else {
            String::new()
        };
        let contract = self.contract_block();
        let contract = if contract.is_empty() { String::new() } else { format!("\nCompletion contract:\n{contract}") };
        format!(
            "[Continuing toward your standing goal]\nGoal: {}{contract}{log_block}{steer}\n\n\
             Continue working toward this goal. Take the next concrete step, then record it with \
             session_goal op=progress (note, verification, error). Verify by execution (run the \
             relevant tests/build/command) before claiming success. \
             Only call session_goal op=complete once you have actually run that verification, and \
             cite what you ran and its result. \
             If you are blocked and need input from the user, say so clearly and stop.",
            self.title
        )
    }
}

/// Record the (possibly failed) supervisor call for this plateau episode and
/// return the continuation prompt with its alternatives injected once.
pub fn apply_supervisor(store: &ControlStore, session_id: &str, alternatives: Option<&str>) -> Result<String> {
    let mut goal = store.get_goal(session_id)?.context("no goal")?;
    goal.sup_turn = Some(goal.turns_used);
    goal.sup_episode = true;
    store.set_goal(session_id, Some(&goal))?;
    Ok(goal.continuation_prompt_with(alternatives.filter(|a| !a.trim().is_empty())))
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeartbeatStatus {
    Active,
    Paused,
    /// `--times N` ticks have fired.
    Done,
}

impl HeartbeatStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Done => "done",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Heartbeat {
    pub id: String,
    pub session_id: String,
    pub source: String,
    pub label: Option<String>,
    pub prompt: String,
    pub status: HeartbeatStatus,
    pub interval_seconds: i64,
    pub created_at_ms: i64,
    pub last_fired_at_ms: i64,
    pub fire_count: i64,
    /// "follow_up" (default; deliver only once the session is idle) or
    /// "steer" (RLM heartbeats only; deliver at the next turn boundary even
    /// while the session is busy, via a soft interrupt).
    pub delivery_mode: String,
    /// `/loop ... --times N`: stop after N ticks (0: no cap).
    pub times: i64,
    /// `/loop ... --until <condition>`: the agent checks it each tick and clears the loop when met.
    pub until: String,
}

impl Heartbeat {
    pub fn new(
        session_id: impl Into<String>,
        prompt: impl Into<String>,
        interval_seconds: i64,
    ) -> Self {
        let now = now_ms();
        let floor = std::env::var("FACTR_HEARTBEAT_MIN_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60i64)
            .max(1);
        let interval = interval_seconds.max(floor);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.into(),
            source: "user".to_string(),
            label: None,
            prompt: prompt.into(),
            status: HeartbeatStatus::Active,
            interval_seconds: interval,
            created_at_ms: now,
            // In test floors (<60s), make the first tick due immediately so e2e
            // can observe a fire without waiting a full minute.
            last_fired_at_ms: if floor < 60 {
                now - interval * 1000
            } else {
                now
            },
            fire_count: 0,
            delivery_mode: "follow_up".to_string(),
            times: 0,
            until: String::new(),
        }
    }

    /// Set with `--times` or `--until`: shown to the desktop as the session's loop, not its heartbeat.
    pub fn is_loop(&self) -> bool {
        self.times > 0 || !self.until.is_empty()
    }

    /// Factr's `loop` snapshot (desktop SessionControlLoop).
    pub fn to_loop_json(&self) -> Value {
        let next = if self.status == HeartbeatStatus::Active { (self.last_fired_at_ms + self.interval_seconds * 1000) as f64 / 1000.0 } else { 0.0 };
        json!({
            "prompt": self.prompt,
            "status": self.status.as_str(),
            "mode": "interval",
            "interval_seconds": self.interval_seconds as f64,
            "current_delay": self.interval_seconds as f64,
            "times": self.times,
            "until": self.until,
            "max_ticks": 0,
            "ticks_fired": self.fire_count,
            "created_at": self.created_at_ms as f64 / 1000.0,
            "last_fired_at": self.last_fired_at_ms as f64 / 1000.0,
            "next_due_at": next,
            "awaiting_response": false,
            "deferred_by_goal": false,
        })
    }

    pub fn is_due(&self, now: i64) -> bool {
        self.status == HeartbeatStatus::Active
            && now.saturating_sub(self.last_fired_at_ms)
                >= self.interval_seconds.saturating_mul(1000)
    }

    /// The desktop's heartbeat chip: exactly the fields `parseHeartbeat` allows.
    pub fn to_control_json(&self) -> Value {
        json!({
            "prompt": self.prompt,
            "status": self.status.as_str(),
            "interval_seconds": self.interval_seconds,
            "created_at": self.created_at_ms as f64 / 1000.0,
            "last_fired_at": self.last_fired_at_ms as f64 / 1000.0,
            "fire_count": self.fire_count,
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "session_id": self.session_id,
            "source": self.source,
            "label": self.label,
            "prompt": self.prompt,
            "status": self.status.as_str(),
            "interval_seconds": self.interval_seconds,
            "created_at_ms": self.created_at_ms,
            "last_fired_at_ms": self.last_fired_at_ms,
            "fire_count": self.fire_count,
            "delivery_mode": self.delivery_mode,
            "times": self.times,
            "until": self.until,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            id: v["id"].as_str()?.to_string(),
            session_id: v["session_id"].as_str()?.to_string(),
            source: v["source"].as_str().unwrap_or("user").to_string(),
            label: v["label"].as_str().map(str::to_string),
            prompt: v["prompt"].as_str()?.to_string(),
            status: HeartbeatStatus::parse(v["status"].as_str()?)?,
            interval_seconds: v["interval_seconds"].as_i64().unwrap_or(60),
            created_at_ms: v["created_at_ms"].as_i64().unwrap_or(0),
            last_fired_at_ms: v["last_fired_at_ms"].as_i64().unwrap_or(0),
            fire_count: v["fire_count"].as_i64().unwrap_or(0),
            delivery_mode: v["delivery_mode"]
                .as_str()
                .unwrap_or("follow_up")
                .to_string(),
            times: v["times"].as_i64().unwrap_or(0),
            until: v["until"].as_str().unwrap_or("").to_string(),
        })
    }

    pub fn fire_prompt(&self) -> String {
        if self.is_loop() {
            let stop = if self.until.is_empty() {
                "If the task is now complete or no longer applicable, say so".to_string()
            } else {
                format!("Stop condition: {}\n\nIf the stop condition is met or the task is no longer applicable, say so", self.until)
            };
            let of = if self.times > 0 { format!("/{}", self.times) } else { String::new() };
            return format!(
                "[/loop wakeup #{}{of}]\nRecurring task: {}\n\nThis is an automatic wakeup from the /loop the user set. Perform the task now against the CURRENT state \
                 (re-check files, processes or services fresh; nothing from earlier ticks still holds). Report concisely what you found or did, with concrete evidence.\n\
                 {stop} and stop the loop by calling the heartbeat tool with op=clear.",
                self.fire_count, self.prompt
            );
        }
        format!(
            "[/heartbeat]\nRecurring check: {}\n\nThis is an automatic heartbeat. Perform the check against current state and report briefly.",
            self.prompt
        )
    }
}

/// Parse `30s` / `5m` / `2h` / `1h30m` into seconds.
pub fn parse_duration_token(token: &str) -> Option<i64> {
    let re = regex_lite_duration(token)?;
    Some(re)
}

fn regex_lite_duration(token: &str) -> Option<i64> {
    let t = token.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    let mut rest = t.as_str();
    let mut total = 0i64;
    let mut matched = false;
    while !rest.is_empty() {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let n: i64 = digits.parse().ok()?;
        rest = &rest[digits.len()..];
        let unit = rest.chars().next()?;
        rest = &rest[unit.len_utf8()..];
        matched = true;
        total += match unit {
            'h' => n * 3600,
            'm' => n * 60,
            's' => n,
            _ => return None,
        };
    }
    if matched && total > 0 {
        Some(total)
    } else {
        None
    }
}

pub struct ControlStore {
    conn: Mutex<Connection>,
}

impl ControlStore {
    pub fn open(home: &Path) -> Result<Self> {
        std::fs::create_dir_all(home).ok();
        let conn = crate::migrate::open(&home.join("factr.db"), SCHEMA).context("opening factr.db")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_cached(home: &Path) -> Result<Arc<Self>> {
        static STORES: OnceLock<Mutex<HashMap<PathBuf, Arc<ControlStore>>>> = OnceLock::new();
        let mut map = STORES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(store) = map.get(home) {
            return Ok(store.clone());
        }
        let store = Arc::new(Self::open(home)?);
        map.insert(home.to_path_buf(), store.clone());
        Ok(store)
    }

    #[cfg(test)]
    pub fn memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        crate::migrate::run(&conn, None)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn get_goal(&self, session_id: &str) -> Result<Option<SessionGoal>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let state: Option<String> = conn
            .query_row(
                "SELECT state FROM session_goals WHERE session_id=?1",
                [session_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(state
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| SessionGoal::from_json(&v)))
    }

    pub fn set_goal(&self, session_id: &str, goal: Option<&SessionGoal>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        match goal {
            None => {
                conn.execute(
                    "DELETE FROM session_goals WHERE session_id=?1",
                    [session_id],
                )?;
            }
            Some(goal) => {
                conn.execute(
                    "INSERT INTO session_goals(session_id, state, updated_at_ms) VALUES (?1, ?2, ?3)
                     ON CONFLICT(session_id) DO UPDATE SET state=excluded.state, updated_at_ms=excluded.updated_at_ms",
                    params![session_id, goal.to_json().to_string(), goal.updated_at_ms],
                )?;
            }
        }
        Ok(())
    }

    pub fn list_heartbeats(&self, session_id: &str) -> Result<Vec<Heartbeat>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT state FROM session_heartbeats WHERE session_id=?1 ORDER BY updated_at_ms",
        )?;
        let rows = stmt.query_map([session_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let s = row?;
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                if let Some(hb) = Heartbeat::from_json(&v) {
                    out.push(hb);
                }
            }
        }
        Ok(out)
    }

    pub fn user_heartbeat(&self, session_id: &str) -> Result<Option<Heartbeat>> {
        Ok(self
            .list_heartbeats(session_id)?
            .into_iter()
            .find(|h| h.source == "user"))
    }

    pub fn upsert_heartbeat(&self, hb: &Heartbeat) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO session_heartbeats(id, session_id, source, state, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET state=excluded.state, updated_at_ms=excluded.updated_at_ms, source=excluded.source",
            params![hb.id, hb.session_id, hb.source, hb.to_json().to_string(), now_ms()],
        )?;
        Ok(())
    }

    pub fn delete_heartbeat(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM session_heartbeats WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn clear_user_heartbeat(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM session_heartbeats WHERE session_id=?1 AND source='user'",
            [session_id],
        )?;
        Ok(())
    }

    /// Active goals parked until their sub-agents finish (a goal parked on a pid is [`Self::pid_waits`]).
    pub fn waiting_sessions(&self) -> Result<Vec<String>> {
        let mut waiting = Vec::new();
        for sid in self.active_sessions()? {
            if self.get_goal(&sid)?.is_some_and(|g| g.status == GoalStatus::Active && g.waiting_on_subagents) {
                waiting.push(sid);
            }
        }
        Ok(waiting)
    }

    /// Active goals parked on a process (`/goal wait <pid>`): `(session, pid)`.
    pub fn pid_waits(&self) -> Result<Vec<(String, i64)>> {
        let mut waits = Vec::new();
        for sid in self.active_sessions()? {
            if let Some(pid) = self.get_goal(&sid)?.filter(|g| g.status == GoalStatus::Active).and_then(|g| g.wait_pid) {
                waits.push((sid, pid));
            }
        }
        Ok(waits)
    }

    /// The sub-agents (or the awaited process) are done or gone: let the goal continue.
    /// `true` if one was waiting.
    pub fn clear_wait(&self, session_id: &str) -> Result<bool> {
        let Some(mut g) = self.get_goal(session_id)?.filter(|g| g.waiting_on_subagents || g.wait_pid.is_some()) else { return Ok(false) };
        g.waiting_on_subagents = false;
        g.wait_pid = None;
        g.wait_reason = None;
        g.updated_at_ms = now_ms();
        self.set_goal(session_id, Some(&g))?;
        Ok(true)
    }

    /// Drop everything the loop keeps for a deleted session.
    pub fn forget_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        for table in ["session_goals", "session_heartbeats"] {
            conn.execute(&format!("DELETE FROM {table} WHERE session_id=?1"), [session_id])?;
        }
        Ok(())
    }

    /// Sessions with an active goal or scheduler: the
    /// gateway driver's work list (empty means it has nothing to wake for).
    pub fn active_sessions(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids = std::collections::BTreeSet::new();
        for table in ["session_goals", "session_heartbeats"] {
            let mut stmt = conn.prepare(&format!("SELECT session_id, state FROM {table}"))?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (id, state) = row?;
                if serde_json::from_str::<Value>(&state).is_ok_and(|v| v["status"] == "active") {
                    ids.insert(id);
                }
            }
        }
        Ok(ids.into_iter().collect())
    }

    /// Full `session.control.read` snapshot the desktop parser accepts.
    pub fn control_snapshot(&self, session_id: &str) -> Result<Value> {
        let goal = self.get_goal(session_id)?;
        let scheduler = self.user_heartbeat(session_id)?;
        let (loop_hb, heartbeat) = match scheduler {
            Some(h) if h.is_loop() => (Some(h), None),
            other => (None, other),
        };
        let mut updated_at = 0.0f64;
        let goal_json = match goal.as_ref() {
            Some(g) if g.status != GoalStatus::Done => {
                updated_at = updated_at.max(g.updated_at_ms as f64 / 1000.0);
                g.to_control_json()
            }
            Some(g) => {
                // Done goals: still return for live chips; hydrate path clears them.
                updated_at = updated_at.max(g.updated_at_ms as f64 / 1000.0);
                g.to_control_json()
            }
            None => Value::Null,
        };
        let loop_json = match loop_hb.as_ref() {
            Some(h) => {
                updated_at = updated_at.max(h.created_at_ms as f64 / 1000.0);
                h.to_loop_json()
            }
            None => Value::Null,
        };
        let hb_json = match heartbeat.as_ref() {
            Some(h) => {
                updated_at = updated_at.max(h.created_at_ms as f64 / 1000.0);
                h.to_control_json()
            }
            None => Value::Null,
        };
        if updated_at == 0.0 {
            updated_at = now_secs_f64();
        }
        let revision = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            goal_json.to_string().hash(&mut h);
            loop_json.to_string().hash(&mut h);
            hb_json.to_string().hash(&mut h);
            format!("{:x}", h.finish())
        };
        Ok(json!({
            "goal": goal_json,
            "loop": loop_json,
            "heartbeat": hb_json,
            "revision": revision,
            "updated_at": updated_at,
        }))
    }
}

/// What the gateway should inject after a completed turn, if anything.
#[derive(Debug, Clone)]
pub enum Continuation {
    Goal(String),
    Heartbeat { id: String, prompt: String },
}

/// After a turn ends: maybe pause for subagents, check gates/budgets, return a follow-up prompt.
pub fn after_turn(
    store: &ControlStore,
    session_id: &str,
    tokens_this_turn: i64,
    subagents_running: bool,
    user_interrupted: bool,
) -> Result<Option<Continuation>> {
    after_turn_in(store, session_id, tokens_this_turn, subagents_running, user_interrupted, None)
}

/// `after_turn` with the session workspace, so ratchet checkpoints can be taken.
pub fn after_turn_in(
    store: &ControlStore,
    session_id: &str,
    tokens_this_turn: i64,
    subagents_running: bool,
    user_interrupted: bool,
    cwd: Option<&Path>,
) -> Result<Option<Continuation>> {
    let obs = turn_obs().lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
    error_retries().lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
    if user_interrupted {
        if let Some(mut goal) = store.get_goal(session_id)? {
            if goal.status == GoalStatus::Active {
                goal.status = GoalStatus::Paused;
                goal.paused_reason = Some("stopped on user input".into());
                goal.updated_at_ms = now_ms();
                store.set_goal(session_id, Some(&goal))?;
            }
        }
        return Ok(None);
    }

    if let Some(mut goal) = store.get_goal(session_id)? {
        if goal.status == GoalStatus::Active {
            goal.turns_used += 1;
            goal.tokens_used += tokens_this_turn;
            let o = obs.unwrap_or_default();
            goal.record_attempt(format!(
                "auto t{} f{} | files={} | ver={} | err={}",
                o.tools,
                o.failed,
                if o.files_changed { "yes" } else { "no" },
                if o.verified { "pass" } else { "none" },
                o.first_err.unwrap_or_default()
            ));
            let line = goal.attempt_log.last().cloned().unwrap_or_default();
            goal.ratchet(&o.scores, o.files_changed, cwd, session_id, &line);
            if !goal.plateaued() {
                goal.sup_episode = false;
            }
            goal.updated_at_ms = now_ms();
            // Parked on sub-agents or on a process (`/goal wait <pid>`): no continuation until it ends.
            goal.waiting_on_subagents = subagents_running;
            if subagents_running || goal.wait_pid.is_some() {
                store.set_goal(session_id, Some(&goal))?;
                return Ok(None);
            }
            // Quality gates run at the turn boundary, before the model may finish: all passing plus a
            // completion the model asked for ends the goal; a failing gate's output is the next prompt.
            let mut gate_note = String::new();
            if !goal.gates.is_empty() {
                let mut failed: Option<(String, String)> = None;
                for gate in &mut goal.gates {
                    let result = run_gate(gate, cwd);
                    if result.passed {
                        gate.attempts = 0;
                    } else {
                        failed = Some((gate.command.clone(), result.output));
                        break;
                    }
                }
                match failed {
                    None => {
                        let commands: Vec<&str> = goal.gates.iter().map(|g| g.command.as_str()).collect();
                        if let Some(verification) = goal.completion_pending.take() {
                            goal.prune_refs(session_id, goal.final_ref());
                            goal.status = GoalStatus::Done;
                            goal.last_verdict = Some("done".into());
                            goal.completion_verification = Some(format!("{verification} | gates passed: {}", commands.join("; ")));
                            store.set_goal(session_id, Some(&goal))?;
                            return Ok(None);
                        }
                        gate_note = format!("\n\nEvery quality gate currently passes ({}). If the goal is achieved, call session_goal op=complete.", commands.join("; "));
                    }
                    Some((command, output)) => {
                        goal.completion_pending = None;
                        let attempts = goal.gates.iter().find(|g| g.command == command).map_or(0, |g| g.attempts);
                        let retries = goal.gates.iter().find(|g| g.command == command).map_or(0, |g| g.max_retries);
                        if attempts > retries {
                            goal.status = GoalStatus::Paused;
                            goal.paused_reason = Some(format!("gate `{command}` still failing after {retries} retries"));
                            goal.last_verdict = Some("blocked".into());
                            store.set_goal(session_id, Some(&goal))?;
                            return Ok(None);
                        }
                        gate_note = format!(
                            "\n\n[Quality gate failed] `{command}`\nGate output (tail):\n```\n{}\n```\nFix the problem so the gate passes; the goal cannot complete until every gate does.",
                            output.chars().take(2000).collect::<String>()
                        );
                    }
                }
            }
            if let Some(reason) = goal.out_of_budget() {
                goal.status = GoalStatus::Paused;
                goal.paused_reason = Some(format!(
                    "{}/{} turns used ({})",
                    goal.turns_used, goal.max_turns, reason
                ));
                goal.last_verdict = Some("blocked".into());
                goal.last_reason = Some(reason.into());
                store.set_goal(session_id, Some(&goal))?;
                return Ok(None);
            }
            store.set_goal(session_id, Some(&goal))?;
            return Ok(Some(Continuation::Goal(format!("{}{gate_note}", goal.continuation_prompt()))));
        }
    }

    Ok(None)
}

/// Goal continuation for a session found active after an engine restart. Unlike `after_turn`
/// it records nothing: no turn ran, so no attempt line, turn count or budget change.
pub fn resume_prompt(store: &ControlStore, session_id: &str) -> Result<Option<Continuation>> {
    Ok(store
        .get_goal(session_id)?
        .filter(|g| g.status == GoalStatus::Active && g.wait_pid.is_none() && g.out_of_budget().is_none())
        .map(|goal| Continuation::Goal(goal.continuation_prompt())))
}

/// Waits before re-sending a goal turn that failed with a transient model error; then it pauses.
const ERROR_BACKOFF: [Duration; 3] = [Duration::from_secs(30), Duration::from_secs(120), Duration::from_secs(600)];

fn error_retries() -> &'static Mutex<HashMap<String, usize>> {
    static R: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// True if `err` contains one of the numeric HTTP `codes` as a whole number.
fn has_code(err: &str, codes: &[&str]) -> bool {
    err.split(|c: char| !c.is_ascii_digit()).any(|n| codes.contains(&n))
}

fn is_auth_error(err: &str) -> bool {
    let e = err.to_lowercase();
    has_code(&e, &["401", "403"]) || ["unauthorized", "forbidden", "invalid api key", "invalid x-api-key", "authentication", "expired token", "no credentials"].iter().any(|k| e.contains(k))
}

fn is_transient_error(err: &str) -> bool {
    let e = err.to_lowercase();
    has_code(&e, &["429", "500", "502", "503", "504", "529"])
        || ["rate limit", "overloaded", "timed out", "timeout", "connection", "network", "temporarily", "unavailable"].iter().any(|k| e.contains(k))
}

#[derive(Debug)]
pub enum ErrorAction {
    /// The goal is now paused; the text says why.
    Paused(String),
    /// Transient failure: re-send `prompt` after `after` if `retry_due(stamp)` still holds.
    Retry { after: Duration, stamp: i64, prompt: Continuation, note: String },
}

/// A turn that ended in a model error. Never counts as a turn, an attempt, or plateau evidence (so no
/// supervisor call): auth and unknown errors pause the goal at once, transient ones (429, 5xx,
/// network) retry after 30 s, 2 min, 10 min and then pause.
pub fn after_error_turn(store: &ControlStore, session_id: &str, error: &str) -> Result<Option<ErrorAction>> {
    turn_obs().lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
    let Some(mut goal) = store.get_goal(session_id)?.filter(|g| g.status == GoalStatus::Active) else {
        error_retries().lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
        return Ok(None);
    };
    let short: String = error.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect();
    let tries = error_retries().lock().unwrap_or_else(|e| e.into_inner()).get(session_id).copied().unwrap_or(0);
    let delay = (!is_auth_error(error) && is_transient_error(error)).then(|| ERROR_BACKOFF.get(tries).copied()).flatten();
    let now = now_ms();
    if let Some(after) = delay {
        error_retries().lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.to_string(), tries + 1);
        let note = format!("model error, retrying in {}s ({}/{}): {short}", after.as_secs(), tries + 1, ERROR_BACKOFF.len());
        goal.updated_at_ms = now;
        store.set_goal(session_id, Some(&goal))?;
        return Ok(Some(ErrorAction::Retry { after, stamp: now, prompt: Continuation::Goal(goal.continuation_prompt()), note }));
    }
    error_retries().lock().unwrap_or_else(|e| e.into_inner()).remove(session_id);
    let reason = format!("model error: {short}");
    goal.status = GoalStatus::Paused;
    goal.paused_reason = Some(reason.clone());
    goal.updated_at_ms = now;
    store.set_goal(session_id, Some(&goal))?;
    Ok(Some(ErrorAction::Paused(reason)))
}

/// Still the goal state `after_error_turn` scheduled the retry for: active and untouched since.
pub fn retry_due(store: &ControlStore, session_id: &str, stamp: i64) -> bool {
    store.get_goal(session_id).ok().flatten().is_some_and(|g| g.status == GoalStatus::Active && g.updated_at_ms == stamp)
}

/// Poll due heartbeats for a session (idle only — caller gates on idle).
pub fn due_heartbeat(store: &ControlStore, session_id: &str) -> Result<Option<Continuation>> {
    let now = now_ms();
    for mut hb in store.list_heartbeats(session_id)? {
        if hb.is_due(now) {
            hb.fire_count += 1;
            hb.last_fired_at_ms = now;
            if hb.times > 0 && hb.fire_count >= hb.times {
                hb.status = HeartbeatStatus::Done; // this is the last tick
            }
            store.upsert_heartbeat(&hb)?;
            return Ok(Some(Continuation::Heartbeat {
                id: hb.id.clone(),
                prompt: hb.fire_prompt(),
            }));
        }
    }
    Ok(None)
}

/// Poll due RLM "steer" heartbeats for a session. Unlike `due_heartbeat`, the
/// caller does NOT need to gate on idle: steer-mode RLM heartbeats are meant
/// to reach a busy session at its next turn boundary via a soft interrupt,
/// not wait for the whole session to go idle. Only `source == "rlm"` and
/// `delivery_mode == "steer"` heartbeats are considered here; plain
/// `follow_up` heartbeats (including the user's own) stay on the
/// idle-only `due_heartbeat` path.
pub fn due_steer_heartbeat(store: &ControlStore, session_id: &str) -> Result<Option<Continuation>> {
    let now = now_ms();
    for mut hb in store.list_heartbeats(session_id)? {
        if hb.source == "rlm" && hb.delivery_mode == "steer" && hb.is_due(now) {
            hb.fire_count += 1;
            hb.last_fired_at_ms = now;
            store.upsert_heartbeat(&hb)?;
            return Ok(Some(Continuation::Heartbeat {
                id: hb.id.clone(),
                prompt: hb.fire_prompt(),
            }));
        }
    }
    Ok(None)
}

// ── Slash command helpers ─────────────────────────────────────────────

/// Split `args` into words, `(raw, value)`: a word that starts with a quote and has its closing
/// quote before the next space is one word whose value drops the quotes (`--gate "cargo test"`);
/// anywhere else a quote or apostrophe is ordinary text.
fn split_words(args: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = args.chars().collect();
    let (mut words, mut i) = (Vec::new(), 0);
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(chars[i], '"' | '\'') {
            let quote = chars[i];
            if let Some(close) = (i + 1..chars.len()).find(|&j| chars[j] == quote && chars.get(j + 1).is_none_or(|c| c.is_whitespace())) {
                words.push((chars[start..=close].iter().collect(), chars[start + 1..close].iter().collect()));
                i = close + 1;
                continue;
            }
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        words.push((word.clone(), word));
    }
    words
}

/// Inline contract lines (`verify: cargo test`, `done when: ...`) split from a goal's text, as in
/// Factr (`goals.parse_contract`): the remaining lines are the headline.
fn extract_contract(text: &str) -> (String, Value) {
    let aliases: [(&str, &str); 22] = [
        ("outcome", "outcome"), ("goal", "outcome"), ("done", "outcome"), ("done when", "outcome"),
        ("verification", "verification"), ("verify", "verification"), ("verified by", "verification"), ("evidence", "verification"), ("proof", "verification"),
        ("constraints", "constraints"), ("constraint", "constraints"), ("preserve", "constraints"), ("must not", "constraints"), ("do not change", "constraints"),
        ("boundaries", "boundaries"), ("boundary", "boundaries"), ("scope", "boundaries"), ("allowed", "boundaries"), ("files", "boundaries"),
        ("stop when", "stop_when"), ("stop_when", "stop_when"), ("blocked", "stop_when"),
    ];
    let mut contract = empty_contract();
    let mut headline = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let key = line.split_once(':').and_then(|(prefix, value)| {
            let key = aliases.iter().find(|(alias, _)| *alias == prefix.trim().to_ascii_lowercase())?.1;
            (!value.trim().is_empty()).then_some((key, value.trim()))
        });
        match key {
            Some((key, value)) => {
                let joined = [contract[key].as_str().unwrap_or(""), value].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
                contract[key] = json!(joined);
            }
            None => headline.push(line),
        }
    }
    (headline.join(" "), contract)
}

/// A `/goal [flags] <text>` command line.
#[derive(Default)]
struct GoalArgs {
    title: String,
    max_turns: Option<i64>,
    token_budget: Option<i64>,
    wall_budget_ms: Option<i64>,
    gates: Vec<String>,
    gate_retries: Option<i64>,
    gate_timeout_ms: Option<i64>,
}

/// Flags: `--budget N` (tokens), `--turns N`, `--timeout-ms N`, `--gate <cmd>` (repeatable),
/// `--gate-retries N`, `--gate-timeout-ms N`; the old `/autonomous` spellings (`--max-tokens`,
/// `--max-turns`, `--max-continuations`) are accepted. A limit may be `unlimited`.
fn parse_goal_args(args: &str) -> GoalArgs {
    let mut out = GoalArgs::default();
    let words = split_words(args);
    let mut title = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let raw = &words[i].0;
        let (flag, inline) = match raw.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.trim_matches(['"', '\'']).to_string())),
            _ => (raw.as_str(), None),
        };
        let known = matches!(
            flag,
            "--budget" | "--token-budget" | "--max-tokens" | "--turns" | "--max-turns" | "--max-continuations" | "--timeout-ms" | "--wall-ms" | "--gate" | "--gate-retries" | "--gate-timeout-ms"
        );
        if !known {
            title.push(raw.clone());
            i += 1;
            continue;
        }
        let val = match inline {
            Some(v) => {
                i += 1;
                Some(v)
            }
            None => {
                let v = words.get(i + 1).map(|(_, v)| v.clone());
                i += 2;
                v
            }
        };
        let number = val.as_deref().and_then(parse_limit);
        match flag {
            "--budget" | "--token-budget" | "--max-tokens" => out.token_budget = number,
            "--turns" | "--max-turns" | "--max-continuations" => out.max_turns = number,
            "--timeout-ms" | "--wall-ms" => out.wall_budget_ms = number,
            "--gate" => out.gates.extend(val.filter(|v| !v.trim().is_empty())),
            "--gate-retries" => out.gate_retries = number,
            _ => out.gate_timeout_ms = number,
        }
    }
    out.title = title.join(" ").trim().to_string();
    out
}

fn parse_limit(s: &str) -> Option<i64> {
    if s.eq_ignore_ascii_case("unlimited") {
        return Some(0);
    }
    s.replace(',', "").replace('_', "").parse().ok()
}

/// Set the session's goal from `/goal` text (flags, gates and inline contract lines as above), with
/// `drafted` filling any contract field the text did not give. The reply is Factr's wording.
pub fn draft_goal(store: &ControlStore, session_id: &str, text: &str, drafted: Option<Value>) -> Result<String> {
    let (headline, mut contract) = extract_contract(text);
    let parsed = parse_goal_args(&headline);
    let inline = contract.as_object().is_some_and(|c| c.values().any(|v| v.as_str().is_some_and(|s| !s.is_empty())));
    if let Some(drafted) = &drafted {
        for key in ["outcome", "verification", "constraints", "boundaries", "stop_when"] {
            if contract[key].as_str().is_none_or(str::is_empty) {
                contract[key] = json!(drafted[key].as_str().unwrap_or("").trim());
            }
        }
    }
    let title = if parsed.title.is_empty() && !parsed.gates.is_empty() {
        "Continue the current task until every quality gate passes.".to_string()
    } else {
        parsed.title.clone()
    };
    if title.is_empty() {
        bail!("Usage: /goal <text> [--budget N] [--turns N] [--gate <command>]");
    }
    let mut goal = SessionGoal::new(title);
    goal.max_turns = parsed.max_turns.map_or(20, |n| if n == 0 { 0 } else { n.max(1) });
    goal.token_budget = parsed.token_budget;
    goal.wall_budget_ms = parsed.wall_budget_ms;
    goal.contract = contract;
    for command in &parsed.gates {
        let mut gate = QualityGate::new(command);
        if let Some(n) = parsed.gate_retries {
            gate.max_retries = n;
        }
        if let Some(n) = parsed.gate_timeout_ms {
            gate.timeout_seconds = (n / 1000).max(1);
        }
        goal.gates.push(gate);
    }
    store.set_goal(session_id, Some(&goal))?;
    let budget = if goal.max_turns == 0 { "no-turn-limit".to_string() } else { format!("{}-turn", goal.max_turns) };
    let mut out = format!("⊙ Goal set ({budget} budget): {}", goal.title);
    for gate in &goal.gates {
        out.push_str(&format!("\n⚿ Gate: $ {} ({} retries, {}s timeout). It must pass before the goal can complete.", gate.command, gate.max_retries, gate.timeout_seconds));
    }
    let block = goal.contract_block();
    if !block.is_empty() {
        out.push_str(&format!("\n{}:\n{block}", if drafted.is_some() { "Drafted completion contract" } else { "Completion contract" }));
    }
    if drafted.is_some() && !inline && !block.is_empty() {
        out.push_str("\nTighten any field by re-setting the goal with inline lines (e.g. verify: <command>), then /goal resume. Use /goal show to review.");
    }
    Ok(out)
}

fn gate_lines(goal: &SessionGoal) -> String {
    if goal.gates.is_empty() {
        return "No quality gates. Add one with /goal gate add <command>.".into();
    }
    goal.gates
        .iter()
        .enumerate()
        .map(|(i, g)| format!("{}. $ {} ({} retries, {}s timeout)", i + 1, g.command, g.max_retries, g.timeout_seconds))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `/goal gate [list | add <command> | remove <N> | clear]`.
fn goal_gate_command(store: &ControlStore, session_id: &str, rest: &str) -> Result<String> {
    let (verb, arg) = match rest.trim().split_once(char::is_whitespace) {
        Some((verb, arg)) => (verb.to_ascii_lowercase(), arg.trim()),
        None => (rest.trim().to_ascii_lowercase(), ""),
    };
    let usage = "Usage: /goal gate [list | add <command> | remove <N> | clear]";
    let mut goal = store.get_goal(session_id)?.filter(|g| g.status != GoalStatus::Done);
    if matches!(verb.as_str(), "" | "list") {
        return Ok(goal.map_or_else(|| "No goal set.".into(), |g| gate_lines(&g)));
    }
    let Some(goal) = goal.as_mut() else { bail!("no goal set; set one with /goal <text> first") };
    let message = match (verb.as_str(), arg.is_empty()) {
        ("add", false) => {
            let gate = QualityGate::new(arg);
            let message = format!("⚿ Gate added: $ {} ({} retries, {}s timeout). It must pass before the goal can complete.", gate.command, gate.max_retries, gate.timeout_seconds);
            goal.gates.push(gate);
            message
        }
        ("remove" | "rm", false) => {
            let n: usize = arg.parse().map_err(|_| anyhow::anyhow!("{usage}"))?;
            if n == 0 || n > goal.gates.len() {
                bail!("no gate {n}; /goal gate lists them");
            }
            format!("✓ Gate removed: $ {}", goal.gates.remove(n - 1).command)
        }
        ("clear", true) => {
            let count = goal.gates.len();
            goal.gates.clear();
            format!("✓ Cleared {count} gate{}.", if count == 1 { "" } else { "s" })
        }
        _ => bail!("{usage}"),
    };
    goal.completion_pending = None;
    goal.updated_at_ms = now_ms();
    store.set_goal(session_id, Some(goal))?;
    Ok(message)
}

/// `/goal` in all its forms. Setting a goal is `[flags] <text>` (see [`draft_goal`]); the rest are
/// `status`, `show`, `pause`, `resume`, `clear` (`stop`, `done`), `complete`, `gate ...`,
/// `wait <pid> [reason]`, `unwait` and `draft <text>` (the gateway drafts the contract with the
/// model first; here it is a plain goal).
pub fn handle_goal_command(store: &ControlStore, session_id: &str, args: &str) -> Result<String> {
    let args = args.trim();
    let (first, rest) = match args.split_once(char::is_whitespace) {
        Some((first, rest)) => (first.to_ascii_lowercase(), rest.trim()),
        None => (args.to_ascii_lowercase(), ""),
    };
    // A whole-word subcommand only: `/goal clear the cache` is a goal.
    let alone = rest.is_empty();
    match first.as_str() {
        "" | "status" if alone => Ok(store
            .get_goal(session_id)?
            .map(|g| g.status_text())
            .unwrap_or_else(|| "No active goal. Set one with /goal <text>.".into())),
        "show" if alone => {
            let Some(goal) = store.get_goal(session_id)? else { return Ok("No active goal. Set one with /goal <text>.".into()) };
            let block = goal.contract_block();
            let mut out = goal.status_text();
            if !block.is_empty() {
                out.push_str(&format!("\n{block}"));
            }
            out.push_str(&format!("\nGates: {}", if goal.gates.is_empty() { "none".to_string() } else { format!("\n{}", gate_lines(&goal)) }));
            Ok(out)
        }
        "clear" | "stop" | "done" if alone => {
            let had = store.get_goal(session_id)?;
            if let Some(goal) = &had {
                goal.prune_refs(session_id, None);
            }
            store.set_goal(session_id, None)?;
            Ok(if had.is_some() { "✓ Goal cleared.".into() } else { "No active goal.".into() })
        }
        "pause" if alone => {
            let mut goal = store
                .get_goal(session_id)?
                .ok_or_else(|| anyhow::anyhow!("No goal to pause."))?;
            goal.status = GoalStatus::Paused;
            goal.paused_reason = Some("paused by user".into());
            goal.updated_at_ms = now_ms();
            store.set_goal(session_id, Some(&goal))?;
            Ok(format!("⏸ Goal paused: {}", goal.title))
        }
        "resume" if alone => {
            let mut goal = store
                .get_goal(session_id)?
                .ok_or_else(|| anyhow::anyhow!("No goal to resume."))?;
            goal.status = GoalStatus::Active;
            goal.paused_reason = None;
            goal.updated_at_ms = now_ms();
            store.set_goal(session_id, Some(&goal))?;
            Ok(format!("▶ Goal resumed: {}", goal.title))
        }
        "complete" if alone => {
            let mut goal = store
                .get_goal(session_id)?
                .ok_or_else(|| anyhow::anyhow!("No goal to complete."))?;
            goal.prune_refs(session_id, goal.final_ref());
            goal.status = GoalStatus::Done;
            goal.last_verdict = Some("done".into());
            goal.updated_at_ms = now_ms();
            store.set_goal(session_id, Some(&goal))?;
            Ok(format!("✓ Goal achieved: {}", goal.title))
        }
        "unwait" if alone => Ok(if store.clear_wait(session_id)? {
            "▶ Wait barrier cleared — goal loop resumes.".into()
        } else {
            "No wait barrier set.".into()
        }),
        "wait" => {
            let Some((pid, reason)) = rest.split_once(char::is_whitespace).map(|(p, r)| (p, r.trim())).or((!rest.is_empty()).then_some((rest, ""))) else {
                bail!("Usage: /goal wait <pid> [reason]");
            };
            let pid: i64 = pid.parse().ok().filter(|p| *p > 0).ok_or_else(|| anyhow::anyhow!("/goal wait: <pid> must be a process id."))?;
            let mut goal = store.get_goal(session_id)?.filter(|g| g.status != GoalStatus::Done).ok_or_else(|| anyhow::anyhow!("no goal set; set one with /goal <text> first"))?;
            goal.wait_pid = Some(pid);
            goal.wait_reason = (!reason.is_empty()).then(|| reason.to_string());
            goal.updated_at_ms = now_ms();
            store.set_goal(session_id, Some(&goal))?;
            let suffix = if reason.is_empty() { String::new() } else { format!(" ({reason})") };
            Ok(format!("⏳ Goal parked on pid {pid}{suffix}. Loop pauses until it exits."))
        }
        "gate" => goal_gate_command(store, session_id, rest),
        "draft" => {
            if rest.is_empty() {
                bail!("Usage: /goal draft <objective in plain language>");
            }
            let set = draft_goal(store, session_id, rest, None)?;
            Ok(format!("{set}\nCouldn't draft a contract (no model available): running as a free-form goal."))
        }
        _ => draft_goal(store, session_id, args, None),
    }
}

/// The first (or resumed) goal turn a `/goal <text>` or `/goal resume` must start, like Factr'
/// `{type: "send", message}`: setting state alone runs nothing. `None` for every other subcommand,
/// or when the goal is not active.
pub fn goal_kickoff(store: &ControlStore, session_id: &str, args: &str) -> Option<String> {
    let first = args.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if matches!(first.as_str(), "" | "status" | "show" | "clear" | "stop" | "done" | "pause" | "complete" | "gate" | "wait" | "unwait") {
        return None;
    }
    store
        .get_goal(session_id)
        .ok()
        .flatten()
        .filter(|g| g.status == GoalStatus::Active && g.wait_pid.is_none() && g.out_of_budget().is_none())
        .map(|g| g.continuation_prompt())
}

/// Seconds in a spelled-out unit (`minute`, `mins`, `hr`, ...).
fn unit_seconds(word: &str) -> Option<i64> {
    match word.to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(1),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(3600),
        _ => None,
    }
}

/// An interval at the start of `words`: `5m`, `1h30m`, `minute`, `2 minutes`. Returns seconds and words used.
fn interval_at(words: &[&str]) -> Option<(i64, usize)> {
    let first = words.first()?;
    if let Some(secs) = parse_duration_token(first) {
        return Some((secs, 1));
    }
    if let Some(unit) = unit_seconds(first).filter(|_| first.len() > 1) {
        return Some((unit, 1));
    }
    let n: i64 = first.parse().ok().filter(|n| *n > 0)?;
    Some((n * unit_seconds(words.get(1)?)?, 2))
}

/// `[every] <interval> <prompt>`, `<prompt> every <interval>`, each with `--times N` and `--until <condition>`
/// anywhere (`--until` takes the rest of the line). `Err` carries the reason and the usage.
fn parse_loop_args(text: &str) -> Result<(i64, String, i64, String)> {
    let (head, until) = match text.find("--until") {
        Some(at) => (&text[..at], text[at + "--until".len()..].trim().to_string()),
        None => (text, String::new()),
    };
    let mut times = 0i64;
    let mut words: Vec<&str> = Vec::new();
    let mut it = head.split_whitespace();
    while let Some(w) = it.next() {
        let n = if w == "--times" { it.next() } else { w.strip_prefix("--times=") };
        match n {
            Some(n) if w.starts_with("--times") => {
                times = n.parse::<i64>().ok().filter(|n| *n >= 1).ok_or_else(|| anyhow::anyhow!("--times expects a positive integer, got {n:?}"))?;
            }
            _ if w == "--times" => bail!("--times expects a positive integer"),
            _ => words.push(w),
        }
    }
    let lead = if words.first().is_some_and(|w| w.eq_ignore_ascii_case("every")) { 1 } else { 0 };
    let (interval, prompt) = if let Some((secs, used)) = interval_at(&words[lead..]) {
        (secs, words[lead + used..].join(" "))
    } else if let Some((secs, at)) = (1..words.len()).rev().find_map(|i| {
        let tail = &words[i + 1..];
        (words[i].eq_ignore_ascii_case("every")).then(|| interval_at(tail).filter(|(_, used)| *used == tail.len()).map(|(s, _)| (s, i))).flatten()
    }) {
        (secs, words[..at].join(" "))
    } else {
        bail!("{LOOP_USAGE}");
    };
    if prompt.trim().is_empty() {
        bail!("{LOOP_USAGE} (the prompt is required)");
    }
    Ok((interval, prompt, times, until))
}

const LOOP_USAGE: &str = "Usage: /loop [every] <interval> <prompt> [--times N] [--until <condition>] | /loop <prompt> every <interval> [--times N] | status | pause | resume | stop. Examples: /loop every 1m check the build --times 3, /loop --times 2 say hello every minute, /loop 30m check CI";

/// `/loop` (aliases `/heartbeat`, `/hb`, `/proactive`): the session's one recurring prompt, kept
/// as a heartbeat. Setting one replaces the last; `status`, `pause`, `resume`, `stop` (`clear`).
pub fn handle_heartbeat_command(
    store: &ControlStore,
    session_id: &str,
    args: &str,
) -> Result<String> {
    let args = args.trim();
    let first = args.split_whitespace().next().unwrap_or("status").to_ascii_lowercase();
    let alone = args.split_whitespace().count() <= 1;
    match first.as_str() {
        "status" | "list" if alone => {
            let list = store.list_heartbeats(session_id)?;
            if list.is_empty() {
                return Ok("No loop. Set one with /loop <interval> <prompt>.".into());
            }
            let lines: Vec<_> = list
                .iter()
                .map(|h| {
                    let mut cap = String::new();
                    if h.times > 0 {
                        cap.push_str(&format!(", {}/{} ticks", h.fire_count, h.times));
                    }
                    if !h.until.is_empty() {
                        cap.push_str(&format!(", until: {}", h.until));
                    }
                    format!(
                        "- [{}] every {} ({}, fired {}{cap}) — {}",
                        h.id.chars().take(8).collect::<String>(),
                        format_duration(h.interval_seconds),
                        h.status.as_str(),
                        h.fire_count,
                        h.prompt
                    )
                })
                .collect();
            Ok(lines.join("\n"))
        }
        "pause" if alone => {
            let mut hb = store
                .user_heartbeat(session_id)?
                .ok_or_else(|| anyhow::anyhow!("No loop to pause."))?;
            hb.status = HeartbeatStatus::Paused;
            store.upsert_heartbeat(&hb)?;
            Ok("Loop paused.".into())
        }
        "resume" if alone => {
            let mut hb = store
                .user_heartbeat(session_id)?
                .ok_or_else(|| anyhow::anyhow!("No loop to resume."))?;
            hb.status = HeartbeatStatus::Active;
            store.upsert_heartbeat(&hb)?;
            Ok("Loop resumed.".into())
        }
        "stop" | "clear" | "cancel" if alone || args.split_whitespace().count() == 2 => {
            if let Some(id) = args.split_whitespace().nth(1) {
                store.delete_heartbeat(id)?;
            } else {
                store.clear_user_heartbeat(session_id)?;
            }
            Ok("Loop stopped.".into())
        }
        _ => {
            let (interval, prompt, times, until) = parse_loop_args(args)?;
            store.clear_user_heartbeat(session_id)?;
            let mut hb = Heartbeat::new(session_id, prompt, interval);
            hb.times = times;
            hb.until = until;
            store.upsert_heartbeat(&hb)?;
            let raised = if hb.interval_seconds > interval { format!(" (the shortest allowed is {})", format_duration(hb.interval_seconds)) } else { String::new() };
            let mut out = format!("Loop set: every {}{raised} — will re-enter this session when idle.", format_duration(hb.interval_seconds));
            if times > 0 {
                out.push_str(&format!(" Stops after {times} tick{}.", if times == 1 { "" } else { "s" }));
            }
            if !hb.until.is_empty() {
                out.push_str(&format!(" Stops when: {}.", hb.until));
            }
            Ok(out)
        }
    }
}

fn format_duration(seconds: i64) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(format!("{s}s"));
    }
    parts.join("")
}

/// Dispatch envelope for `session.control` actions.
pub fn control_action(
    store: &ControlStore,
    session_id: &str,
    action: &str,
    args: &Value,
) -> Result<(Value, Value)> {
    let message = match action {
        "goal.clear" => handle_goal_command(store, session_id, "clear")?,
        "goal.pause" => handle_goal_command(store, session_id, "pause")?,
        "goal.resume" => handle_goal_command(store, session_id, "resume")?,
        "goal.unwait" => {
            store.clear_wait(session_id)?;
            "Goal wait cleared.".into()
        }
        "subgoal.add" => {
            let text = args["text"].as_str().unwrap_or_default();
            if let Some(mut g) = store.get_goal(session_id)? {
                g.subgoals.push(text.to_string());
                g.updated_at_ms = now_ms();
                store.set_goal(session_id, Some(&g))?;
                format!("Added subgoal: {text}")
            } else {
                bail!("No active goal");
            }
        }
        "subgoal.remove" => {
            let index = args["index"].as_u64().unwrap_or(0) as usize;
            if let Some(mut g) = store.get_goal(session_id)? {
                if index == 0 || index > g.subgoals.len() {
                    bail!("Invalid subgoal index");
                }
                let removed = g.subgoals.remove(index - 1);
                g.updated_at_ms = now_ms();
                store.set_goal(session_id, Some(&g))?;
                format!("Removed subgoal: {removed}")
            } else {
                bail!("No active goal");
            }
        }
        "subgoal.clear" => {
            if let Some(mut g) = store.get_goal(session_id)? {
                g.subgoals.clear();
                g.updated_at_ms = now_ms();
                store.set_goal(session_id, Some(&g))?;
            }
            "Subgoals cleared.".into()
        }
        "heartbeat.clear" => handle_heartbeat_command(store, session_id, "clear")?,
        "heartbeat.pause" => handle_heartbeat_command(store, session_id, "pause")?,
        "heartbeat.resume" => handle_heartbeat_command(store, session_id, "resume")?,
        "loop.pause" => handle_heartbeat_command(store, session_id, "pause")?,
        "loop.resume" => handle_heartbeat_command(store, session_id, "resume")?,
        "loop.stop" => handle_heartbeat_command(store, session_id, "clear")?,
        other => bail!("Unknown session.control action: {other}"),
    };
    let control = store.control_snapshot(session_id)?;
    if action == "goal.resume" {
        if let Some(prompt) = goal_kickoff(store, session_id, "resume") {
            let dispatch = json!({
                "type": "send",
                "output": message,
                "notice": message,
                "message": prompt,
                "display": "Resume goal",
            });
            return Ok((control, dispatch));
        }
    }
    let dispatch = json!({
        "type": "exec",
        "output": message,
        "notice": null,
        "message": null,
        "display": null,
    });
    Ok((control, dispatch))
}

#[cfg(test)]
mod tests {
    #[test]
    fn run_command_runs_through_the_platform_shell_and_reports_the_exit_code() {
        // Valid in both `sh -c` and `cmd.exe /D /S /C`.
        let r = run_command("echo gate-ok", None, Duration::from_secs(20));
        assert!(r.passed && r.exit_code == 0 && r.output.contains("gate-ok"), "{r:?}");
        let r = run_command("exit 3", None, Duration::from_secs(20));
        assert!(!r.passed && r.exit_code == 3, "{r:?}");
    }

    #[cfg(unix)]
    #[test]
    fn run_command_uses_cwd_and_drains_verbose_output() {
        let dir = std::env::temp_dir().join(format!("gate-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = run_command("pwd; head -c 400000 /dev/zero | tr '\\0' x; echo END; exit 3", Some(&dir), Duration::from_secs(20));
        assert_eq!(r.exit_code, 3);
        assert!(r.output.contains("END"));
        assert!(r.output.len() <= 3000);
        let r = run_command("pwd", Some(&dir), Duration::from_secs(20));
        assert!(r.passed && r.output.trim().ends_with(dir.file_name().unwrap().to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn run_command_timeout_kills_grandchildren() {
        let marker = format!("sleep {}", 300 + std::process::id() % 97);
        let t = Instant::now();
        let r = run_command(&format!("{marker} & wait"), None, Duration::from_millis(500));
        assert_eq!(r.exit_code, 124);
        assert!(t.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(200));
        let ps = Command::new("pgrep").args(["-f", &marker]).output().unwrap();
        assert!(String::from_utf8_lossy(&ps.stdout).trim().is_empty(), "grandchild survived");
    }

    use super::*;

    #[test]
    fn active_sessions_lists_only_active_goals_loops_and_heartbeats() {
        let store = ControlStore::memory().unwrap();
        assert!(store.active_sessions().unwrap().is_empty());
        store.set_goal("g", Some(&SessionGoal::new("ship"))).unwrap();
        let mut paused = SessionGoal::new("later");
        paused.status = GoalStatus::Paused;
        store.set_goal("paused", Some(&paused)).unwrap();
        store.upsert_heartbeat(&Heartbeat::new("h", "check", 600)).unwrap();
        let mut off = Heartbeat::new("off", "check", 600);
        off.status = HeartbeatStatus::Paused;
        store.upsert_heartbeat(&off).unwrap();
        assert_eq!(store.active_sessions().unwrap(), ["g", "h"]);
    }

    #[test]
    fn goal_stops_at_turn_budget() {
        let store = ControlStore::memory().unwrap();
        let mut goal = SessionGoal::new("ship it");
        goal.max_turns = 2;
        store.set_goal("s1", Some(&goal)).unwrap();
        let c1 = after_turn(&store, "s1", 10, false, false).unwrap();
        assert!(matches!(c1, Some(Continuation::Goal(_))));
        let c2 = after_turn(&store, "s1", 10, false, false).unwrap();
        assert!(c2.is_none());
        let g = store.get_goal("s1").unwrap().unwrap();
        assert_eq!(g.status, GoalStatus::Paused);
        assert!(g.paused_reason.as_deref().unwrap().contains("turn"));
    }

    #[test]
    fn heartbeat_persists_and_fires_once() {
        let store = ControlStore::memory().unwrap();
        let mut hb = Heartbeat::new("s1", "check CI", 60);
        hb.last_fired_at_ms = now_ms() - 61_000;
        store.upsert_heartbeat(&hb).unwrap();
        let due = due_heartbeat(&store, "s1").unwrap().unwrap();
        assert!(matches!(due, Continuation::Heartbeat { .. }));
        let loaded = store.user_heartbeat("s1").unwrap().unwrap();
        assert_eq!(loaded.fire_count, 1);
        // Not immediately due again
        assert!(due_heartbeat(&store, "s1").unwrap().is_none());
    }

    #[test]
    fn control_read_shape_has_exact_fields() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("s1", Some(&SessionGoal::new("x"))).unwrap();
        let mut looped = Heartbeat::new("s1", "ping", 60);
        looped.times = 3;
        store.upsert_heartbeat(&looped).unwrap();
        let snap = store.control_snapshot("s1").unwrap();
        assert!(snap["goal"].is_object());
        assert!(snap["loop"].is_object(), "a scheduler with --times is the loop");
        assert!(snap["heartbeat"].is_null(), "one scheduler is shown once");
        let loop_snap = snap.clone();
        store.clear_user_heartbeat("s1").unwrap();
        store.upsert_heartbeat(&Heartbeat::new("s1", "ping", 60)).unwrap();
        let snap = store.control_snapshot("s1").unwrap();
        assert!(snap["loop"].is_null() && snap["heartbeat"].is_object(), "a plain one is the heartbeat");
        let (heartbeat, snap) = (snap["heartbeat"].clone(), loop_snap);
        assert!(snap["revision"].is_string());
        assert!(snap["updated_at"].as_f64().is_some());
        // Goal required fields
        for key in [
            "title",
            "status",
            "turns_used",
            "max_turns",
            "contract",
            "subgoals",
            "gates",
        ] {
            assert!(snap["goal"].get(key).is_some(), "missing goal.{key}");
        }
        for key in [
            "prompt",
            "status",
            "mode",
            "interval_seconds",
            "current_delay",
            "times",
            "until",
            "max_ticks",
            "ticks_fired",
            "created_at",
            "last_fired_at",
            "next_due_at",
            "awaiting_response",
            "deferred_by_goal",
        ] {
            assert!(snap["loop"].get(key).is_some(), "missing loop.{key}");
        }
        for key in [
            "prompt",
            "status",
            "interval_seconds",
            "created_at",
            "last_fired_at",
            "fire_count",
        ] {
            assert!(heartbeat.get(key).is_some(), "missing heartbeat.{key}");
        }
    }

    /// The desktop parses each snapshot strictly: a key outside its allowlist makes the whole
    /// `session.control.read` answer "invalid" and the chat shows "Session controls unavailable".
    #[test]
    fn control_read_has_no_field_the_desktop_parser_refuses() {
        fn only(value: &Value, allowed: &[&str], what: &str) {
            let extra: Vec<&String> = value.as_object().unwrap().keys().filter(|k| !allowed.contains(&k.as_str())).collect();
            assert!(extra.is_empty(), "{what} carries {extra:?}");
        }
        let store = ControlStore::memory().unwrap();
        let mut goal = SessionGoal::new("x");
        goal.attempt_log = vec!["tried".into()];
        goal.completion_verification = Some("ran it".into());
        goal.paused_reason = Some("p".into());
        goal.last_verdict = Some("blocked".into());
        goal.last_reason = Some("r".into());
        store.set_goal("s1", Some(&goal)).unwrap();
        let snap = store.control_snapshot("s1").unwrap();
        only(&snap, &["goal", "loop", "heartbeat", "revision", "updated_at"], "snapshot");
        only(
            &snap["goal"],
            &["title", "status", "turns_used", "max_turns", "contract", "subgoals", "gates", "created_at", "updated_at", "paused_reason", "last_verdict", "last_reason", "wait_barrier"],
            "goal",
        );
        only(&snap["goal"]["contract"], &["outcome", "verification", "constraints", "boundaries", "stop_when"], "goal.contract");
        let mut hb = Heartbeat::new("s1", "ping", 60);
        hb.label = Some("named".into());
        store.upsert_heartbeat(&hb).unwrap();
        let snap = store.control_snapshot("s1").unwrap();
        only(&snap["heartbeat"], &["prompt", "status", "interval_seconds", "created_at", "last_fired_at", "fire_count"], "heartbeat");
        hb.times = 2;
        store.upsert_heartbeat(&hb).unwrap();
        let snap = store.control_snapshot("s1").unwrap();
        only(
            &snap["loop"],
            &["prompt", "status", "mode", "interval_seconds", "current_delay", "times", "until", "max_ticks", "ticks_fired", "created_at", "last_fired_at", "next_due_at", "awaiting_response", "deferred_by_goal", "paused_reason", "last_stop_reason"],
            "loop",
        );
    }

    #[test]
    fn goal_flags_accept_the_old_autonomous_spellings_and_quoted_gates() {
        let store = ControlStore::memory().unwrap();
        let said = handle_goal_command(&store, "s", "fix the build --max-turns 4 --max-tokens 9,000 --gate \"cargo test -q\" --gate-retries 1 --gate-timeout-ms 5000 --timeout-ms 60000").unwrap();
        assert!(said.contains("4-turn budget") && said.contains("fix the build") && said.contains("cargo test -q"), "{said}");
        let g = store.get_goal("s").unwrap().unwrap();
        assert_eq!((g.title.as_str(), g.max_turns, g.token_budget, g.wall_budget_ms), ("fix the build", 4, Some(9000), Some(60000)));
        assert_eq!((g.gates[0].command.as_str(), g.gates[0].max_retries, g.gates[0].timeout_seconds), ("cargo test -q", 1, 5));
        // Gates alone are enough: the goal is "until they pass"; "unlimited" lifts the turn cap.
        handle_goal_command(&store, "g", "--gate true --turns unlimited").unwrap();
        let g = store.get_goal("g").unwrap().unwrap();
        assert!(g.title.contains("quality gate") && g.max_turns == 0 && g.gates.len() == 1);
        assert!(handle_goal_command(&store, "e", "--turns 3").is_err(), "no text and no gate is a usage error");
        // An apostrophe in the text is text, not a quote.
        handle_goal_command(&store, "q", "don't break the API --turns 2").unwrap();
        assert_eq!(store.get_goal("q").unwrap().unwrap().title, "don't break the API");
    }

    #[test]
    fn goal_subcommands_gate_show_wait_unwait_draft_and_whole_word_only() {
        let store = ControlStore::memory().unwrap();
        assert!(handle_goal_command(&store, "s", "gate add true").is_err(), "a gate needs a goal");
        handle_goal_command(&store, "s", "ship it\nverify: cargo test\nstop when: the schema changes").unwrap();
        let shown = handle_goal_command(&store, "s", "show").unwrap();
        assert!(shown.contains("Verification: cargo test") && shown.contains("Stop when blocked: the schema changes") && shown.contains("Gates: none"), "{shown}");
        assert!(handle_goal_command(&store, "s", "gate add cargo test -q").unwrap().contains("Gate added: $ cargo test -q"));
        assert!(handle_goal_command(&store, "s", "gate").unwrap().contains("1. $ cargo test -q (3 retries, 120s timeout)"));
        assert!(handle_goal_command(&store, "s", "gate remove 9").is_err());
        assert!(handle_goal_command(&store, "s", "gate rm 1").unwrap().contains("Gate removed"));
        assert!(handle_goal_command(&store, "s", "gate clear").unwrap().contains("Cleared 0 gates"));
        // Wait barrier on a process, then lift it.
        assert!(handle_goal_command(&store, "s", "wait nope").is_err() && handle_goal_command(&store, "s", "wait").is_err());
        assert!(handle_goal_command(&store, "s", "wait 4242 the build").unwrap().contains("parked on pid 4242 (the build)"));
        assert!(store.get_goal("s").unwrap().unwrap().status_text().contains("parked on pid 4242"));
        assert_eq!(store.pid_waits().unwrap(), [("s".to_string(), 4242)]);
        assert!(after_turn(&store, "s", 0, false, false).unwrap().is_none(), "no continuation while parked");
        assert!(resume_prompt(&store, "s").unwrap().is_none() && goal_kickoff(&store, "s", "resume").is_none());
        assert!(handle_goal_command(&store, "s", "unwait").unwrap().contains("Wait barrier cleared"));
        assert_eq!(handle_goal_command(&store, "s", "unwait").unwrap(), "No wait barrier set.");
        assert!(matches!(after_turn(&store, "s", 0, false, false).unwrap(), Some(Continuation::Goal(p)) if p.contains("Completion contract") && p.contains("cargo test")));
        // `clear` and `stop` are whole words: `/goal clear the cache` is a goal.
        handle_goal_command(&store, "c", "clear the cache").unwrap();
        assert_eq!(store.get_goal("c").unwrap().unwrap().title, "clear the cache");
        assert_eq!(handle_goal_command(&store, "c", "stop").unwrap(), "✓ Goal cleared.");
        assert_eq!(handle_goal_command(&store, "c", "clear").unwrap(), "No active goal.");
        // `draft` without a model is a plain goal and says so; with a drafted contract the fields are set.
        assert!(handle_goal_command(&store, "d", "draft").is_err());
        assert!(handle_goal_command(&store, "d", "draft port the parser").unwrap().contains("free-form goal"));
        let said = draft_goal(&store, "d2", "port the parser", Some(json!({ "outcome": "parser ported", "verification": "cargo test parser" }))).unwrap();
        assert!(said.contains("Drafted completion contract") && said.contains("Verification: cargo test parser"), "{said}");
    }

    #[test]
    fn quality_gates_decide_when_a_goal_can_complete() {
        use crate::agent_loop_host::goal_host;
        let store = ControlStore::memory().unwrap();
        handle_goal_command(&store, "s", "do it --gate true").unwrap();
        // Passing gates alone do not end the goal: the model must ask for completion.
        assert!(matches!(after_turn(&store, "s", 0, false, false).unwrap(), Some(Continuation::Goal(p)) if p.contains("Every quality gate currently passes")));
        let asked: Value = serde_json::from_str(&goal_host(&store, "s", r#"{"op":"complete","verification":"ran it, ok"}"#).unwrap()).unwrap();
        assert_eq!(asked["goal"]["status"], "active");
        assert!(asked["completion_pending"].as_str().unwrap().contains("gates"));
        assert!(after_turn(&store, "s", 0, false, false).unwrap().is_none(), "gates passed: the goal ends");
        let g = store.get_goal("s").unwrap().unwrap();
        assert_eq!(g.status, GoalStatus::Done);
        assert!(g.completion_verification.unwrap().contains("gates passed: true"));
        // A failing gate refuses the pending completion and its output is the next prompt; retries run out.
        handle_goal_command(&store, "f", "do it --gate \"echo broken >&2; exit 1\" --gate-retries 1").unwrap();
        goal_host(&store, "f", r#"{"op":"complete","verification":"it works"}"#).unwrap();
        match after_turn(&store, "f", 0, false, false).unwrap() {
            Some(Continuation::Goal(p)) => assert!(p.contains("[Quality gate failed]") && p.contains("broken"), "{p}"),
            other => panic!("{other:?}"),
        }
        let g = store.get_goal("f").unwrap().unwrap();
        assert!(g.status == GoalStatus::Active && g.completion_pending.is_none());
        assert!(after_turn(&store, "f", 0, false, false).unwrap().is_none());
        let g = store.get_goal("f").unwrap().unwrap();
        assert!(g.status == GoalStatus::Paused && g.paused_reason.unwrap().contains("still failing after 1 retries"));
    }

    #[test]
    fn a_loop_is_a_heartbeat_with_times_until_and_one_per_session() {
        let store = ControlStore::memory().unwrap();
        for bad in ["", "five minutes ping", "5m", "5m ping --times 0", "5m ping --times x"] {
            if bad.is_empty() {
                continue;
            }
            assert!(handle_heartbeat_command(&store, "s", bad).is_err(), "{bad}");
        }
        for (text, want) in [
            ("every 1m Reply with exactly the word hello --times 2", (60, "Reply with exactly the word hello", 2)),
            ("--times 2 hello every minute", (60, "hello", 2)),
            ("hello every 2 minutes --times 4", (120, "hello", 4)),
            ("30m check CI", (1800, "check CI", 0)),
            ("every minute ping", (60, "ping", 0)),
            ("every 1h30m watch it --times=3", (5400, "watch it", 3)),
        ] {
            let (secs, prompt, times, _) = parse_loop_args(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!((secs, prompt.as_str(), times), want, "{text}");
        }
        let err = handle_heartbeat_command(&store, "s", "soon hello --times 2").unwrap_err().to_string();
        assert!(err.contains("Usage: /loop") && err.contains("Examples"), "{err}");
        let said = handle_heartbeat_command(&store, "s", "every 1m check CI --times 2 --until the build is green").unwrap();
        assert!(said.contains("Stops after 2 ticks") && said.contains("Stops when: the build is green"), "{said}");
        // Setting another replaces it: one scheduler per session.
        handle_heartbeat_command(&store, "s", "5m watch the deploy --until it is live").unwrap();
        assert_eq!(store.list_heartbeats("s").unwrap().len(), 1);
        assert!(handle_heartbeat_command(&store, "s", "10s tick --times 2").unwrap().contains("the shortest allowed is 1m"));
        let hb = store.user_heartbeat("s").unwrap().unwrap();
        assert_eq!((hb.times, hb.until.as_str(), hb.is_loop()), (2, "", true));
        // Two ticks, then it is done and out of the work list; the last tick still fires.
        let mut hb = hb;
        hb.last_fired_at_ms = now_ms() - 120_000;
        store.upsert_heartbeat(&hb).unwrap();
        let first = due_heartbeat(&store, "s").unwrap();
        assert!(matches!(first, Some(Continuation::Heartbeat { ref prompt, .. }) if prompt.contains("[/loop wakeup #1/2]") && prompt.contains("op=clear")));
        let mut hb = store.user_heartbeat("s").unwrap().unwrap();
        hb.last_fired_at_ms = now_ms() - 120_000;
        store.upsert_heartbeat(&hb).unwrap();
        assert!(due_heartbeat(&store, "s").unwrap().is_some());
        assert_eq!(store.user_heartbeat("s").unwrap().unwrap().status, HeartbeatStatus::Done);
        assert!(store.active_sessions().unwrap().is_empty());
        assert!(handle_heartbeat_command(&store, "s", "status").unwrap().contains("2/2 ticks"));
        // pause / resume / stop, and the old heartbeat spellings still work.
        handle_heartbeat_command(&store, "t", "every 5m check CI").unwrap();
        assert!(!store.user_heartbeat("t").unwrap().unwrap().is_loop(), "no --times or --until: a plain heartbeat");
        assert_eq!(handle_heartbeat_command(&store, "t", "pause").unwrap(), "Loop paused.");
        assert_eq!(handle_heartbeat_command(&store, "t", "resume").unwrap(), "Loop resumed.");
        assert_eq!(handle_heartbeat_command(&store, "t", "stop").unwrap(), "Loop stopped.");
        assert!(store.list_heartbeats("t").unwrap().is_empty());
        let (_, d) = control_action(&store, "s", "loop.stop", &json!({})).unwrap();
        assert_eq!(d["output"], "Loop stopped.");
    }

    #[test]
    fn parse_duration_and_slash_goal() {
        assert_eq!(parse_duration_token("5m"), Some(300));
        assert_eq!(parse_duration_token("1h30m"), Some(5400));
        let store = ControlStore::memory().unwrap();
        let msg = handle_goal_command(&store, "s1", "--turns 3 ship the feature").unwrap();
        assert!(msg.contains("Goal set"));
        assert!(msg.contains("ship the feature"));
        let g = store.get_goal("s1").unwrap().unwrap();
        assert_eq!(g.max_turns, 3);
    }

    #[test]
    fn waiting_on_subagents_pauses_continuation() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("s1", Some(&SessionGoal::new("x"))).unwrap();
        let c = after_turn(&store, "s1", 0, true, false).unwrap();
        assert!(c.is_none());
        assert!(store.get_goal("s1").unwrap().unwrap().waiting_on_subagents);
    }

    #[test]
    fn clearing_the_wait_lets_the_parked_goal_continue() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("p", Some(&SessionGoal::new("x"))).unwrap();
        assert!(after_turn(&store, "p", 0, true, false).unwrap().is_none());
        assert_eq!(store.waiting_sessions().unwrap(), ["p"]);
        assert!(store.clear_wait("p").unwrap());
        assert!(store.waiting_sessions().unwrap().is_empty());
        assert!(!store.clear_wait("p").unwrap());
        assert!(matches!(resume_prompt(&store, "p").unwrap(), Some(Continuation::Goal(_))));
    }

    #[test]
    fn user_interrupt_pauses_goal() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("s1", Some(&SessionGoal::new("x"))).unwrap();
        let c = after_turn(&store, "s1", 0, false, true).unwrap();
        assert!(c.is_none());
        assert_eq!(
            store.get_goal("s1").unwrap().unwrap().status,
            GoalStatus::Paused
        );
    }

    #[test]
    fn goal_set_and_resume_start_a_turn_and_the_budget_and_clear_stop_it() {
        let store = ControlStore::memory().unwrap();
        let said = handle_goal_command(&store, "g", "--turns 2 write goal_demo.py").unwrap();
        assert!(said.contains("Goal set"));
        let kick = goal_kickoff(&store, "g", "--turns 2 write goal_demo.py").expect("set starts the first turn");
        assert!(kick.contains("write goal_demo.py"));
        assert!(goal_kickoff(&store, "g", "status").is_none() && goal_kickoff(&store, "g", "pause").is_none());
        // Budget: two turns, then it pauses and sends nothing more.
        assert!(after_turn(&store, "g", 0, false, false).unwrap().is_some());
        assert!(after_turn(&store, "g", 0, false, false).unwrap().is_none());
        let g = store.get_goal("g").unwrap().unwrap();
        assert_eq!((&g.status, g.turns_used), (&GoalStatus::Paused, 2));
        assert!(store.active_sessions().unwrap().is_empty());
        // Resume of a paused goal (control action) is a send; /goal clear stops it for good.
        let mut g = g;
        g.max_turns = 5;
        store.set_goal("g", Some(&g)).unwrap();
        let (_, d) = control_action(&store, "g", "goal.resume", &json!({})).unwrap();
        assert_eq!(d["type"], "send");
        assert!(d["message"].as_str().unwrap().contains("write goal_demo.py"));
        handle_goal_command(&store, "g", "clear").unwrap();
        assert!(goal_kickoff(&store, "g", "resume").is_none());
        assert!(after_turn(&store, "g", 0, false, false).unwrap().is_none());
    }

    #[test]
    fn attempt_log_is_capped_and_truncated() {
        let mut goal = SessionGoal::new("x");
        for i in 0..12 {
            goal.record_attempt(format!("attempt number {i}"));
        }
        assert_eq!(goal.attempt_log.len(), MAX_ATTEMPT_LOG);
        // Oldest entries are dropped, newest kept.
        assert_eq!(goal.attempt_log.first().unwrap(), "attempt number 4");
        assert_eq!(goal.attempt_log.last().unwrap(), "attempt number 11");

        let long = "x".repeat(200);
        goal.record_attempt(long);
        let last = goal.attempt_log.last().unwrap();
        assert!(last.chars().count() <= MAX_ATTEMPT_LEN);
        assert!(last.ends_with('…'));
    }

    #[test]
    fn error_turns_back_off_then_pause_and_never_count() {
        let dir = std::env::temp_dir().join(format!("errturn-{}", std::process::id()));
        let store = ControlStore::open(&dir).unwrap();
        store.set_goal("e1", Some(&SessionGoal::new("g"))).unwrap();
        // Auth errors pause at once.
        let Some(ErrorAction::Paused(why)) = after_error_turn(&store, "e1", "HTTP 401 Unauthorized").unwrap() else { panic!() };
        assert!(why.contains("401"));
        let g = store.get_goal("e1").unwrap().unwrap();
        assert_eq!((g.status, g.turns_used, g.attempt_log.len()), (GoalStatus::Paused, 0, 0));
        // Transient errors retry 30 s, 2 min, 10 min, then pause; a healthy turn resets the count.
        store.set_goal("e2", Some(&SessionGoal::new("g"))).unwrap();
        for secs in [30, 120, 600] {
            let Some(ErrorAction::Retry { after, stamp, .. }) = after_error_turn(&store, "e2", "429 rate limit").unwrap() else { panic!() };
            assert_eq!(after.as_secs(), secs);
            assert!(retry_due(&store, "e2", stamp));
        }
        assert!(matches!(after_error_turn(&store, "e2", "503 overloaded").unwrap(), Some(ErrorAction::Paused(_))));
        assert_eq!(store.get_goal("e2").unwrap().unwrap().turns_used, 0);
        // Resume records nothing.
        store.set_goal("e3", Some(&SessionGoal::new("g"))).unwrap();
        assert!(resume_prompt(&store, "e3").unwrap().is_some());
        assert_eq!(store.get_goal("e3").unwrap().unwrap().turns_used, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn plateau_detection_needs_no_model_call() {
        let mut goal = SessionGoal::new("x");
        assert!(!goal.plateaued(), "empty log is never a plateau");
        goal.record_attempt("fail: same build error");
        goal.record_attempt("pass: different step");
        assert!(!goal.plateaued(), "two distinct entries is not a plateau");
        goal.record_attempt("fail: same build error");
        goal.record_attempt("fail: same build error");
        goal.record_attempt("fail: same build error");
        assert!(
            goal.plateaued(),
            "3 identical consecutive attempts is a plateau"
        );
    }

    #[test]
    fn continuation_prompt_injects_attempt_log_and_steer_on_plateau() {
        let mut goal = SessionGoal::new("ship it");
        let prompt = goal.continuation_prompt();
        assert!(!prompt.contains("Recent attempts"));
        assert!(!prompt.contains("Plateau detected"));

        goal.record_attempt("fail: lint error");
        let prompt = goal.continuation_prompt();
        assert!(prompt.contains("Recent attempts"));
        assert!(prompt.contains("fail: lint error"));
        assert!(!prompt.contains("Plateau detected"));

        goal.record_attempt("fail: lint error");
        goal.record_attempt("fail: lint error");
        let prompt = goal.continuation_prompt();
        assert!(prompt.contains("Plateau detected"));
        assert!(prompt.contains("different strategy"));
    }

    #[test]
    fn failed_verification_never_ends_the_goal_only_budget_does() {
        let store = ControlStore::memory().unwrap();
        let mut goal = SessionGoal::new("x");
        goal.max_turns = 2;
        goal.record_attempt("fail: verification failed");
        store.set_goal("s1", Some(&goal)).unwrap();
        // A failed verification is just an attempt-log entry; the goal keeps
        // going until budget exhaustion, completion, or a user cancel.
        let c1 = after_turn(&store, "s1", 0, false, false).unwrap();
        assert!(matches!(c1, Some(Continuation::Goal(_))));
        assert_eq!(
            store.get_goal("s1").unwrap().unwrap().status,
            GoalStatus::Active
        );
        // Budget exhaustion is the actual stop condition, and it keeps the
        // attempt log rather than discarding it.
        let c2 = after_turn(&store, "s1", 0, false, false).unwrap();
        assert!(c2.is_none());
        let paused = store.get_goal("s1").unwrap().unwrap();
        assert_eq!(paused.status, GoalStatus::Paused);
        assert_eq!(paused.attempt_log[0], "fail: verification failed");
    }

    #[test]
    fn verification_is_matched_by_program_and_subcommand_not_substring() {
        use crate::goal_ratchet::is_verify_command as v;
        for yes in [
            "cargo test -p foo", "cargo +nightly clippy --all", "cd app && cargo build 2>&1 | tail", "RUST_LOG=x cargo check",
            "npm test", "npm run test:unit", "pnpm run build", "yarn lint", "yarn test --watch=false", "pytest -x", "python3 -m pytest tests/",
            "go test ./...", "npx jest", "pnpm exec vitest run", "vitest", "tsc --noEmit", "make test", "make check", "./gradlew test", "mvn -q test",
            "swift test", "bun test", "deno test -A", "echo hi; cargo test", "sudo -n go test",
        ] {
            assert!(v(yes), "should verify: {yes}");
        }
        for no in [
            "echo test", "git checkout main", "ls tests", "cat build.log", "git commit -m 'cargo test'", "echo 'a && cargo test'",
            "npm install", "cargo fmt", "python script.py test", "grep -r lint src", "make install", "cargo run -- test",
        ] {
            assert!(!v(no), "should not verify: {no}");
        }
    }

    #[test]
    fn a_goal_whose_verification_only_ever_failed_cannot_complete_on_free_text() {
        use crate::agent_loop_host::goal_host;
        let store = ControlStore::memory().unwrap();
        store.set_goal("f", Some(&SessionGoal::new("x"))).unwrap();
        let cargo = json!({"command": "cargo test"});
        let complete = r#"{"op":"complete","verification":"I checked it by hand"}"#;
        run_auto_turn(&store, "f", &[("edit", json!({}), "ok"), ("bash", cargo.clone(), "1 failed\n\nExit code: 101")]);
        let goal = store.get_goal("f").unwrap().unwrap();
        assert!(goal.best.is_empty(), "a failing run never reaches best");
        assert_eq!((goal.verify_runs, goal.verify_ok), (1, false));
        assert!(goal_host(&store, "f", complete).is_err(), "free text after only failures");
        run_auto_turn(&store, "f", &[("bash", cargo, "test result: ok. 3 passed; 0 failed\n\nExit code: 0")]);
        assert!(store.get_goal("f").unwrap().unwrap().verify_ok);
        assert!(goal_host(&store, "f", complete).is_ok(), "a recorded pass unlocks completion");
    }

    #[test]
    fn a_non_verification_command_does_not_lock_completion() {
        use crate::agent_loop_host::goal_host;
        let store = ControlStore::memory().unwrap();
        store.set_goal("n", Some(&SessionGoal::new("x"))).unwrap();
        run_auto_turn(&store, "n", &[("bash", json!({"command": "echo test"}), "test\n\nExit code: 0")]);
        assert_eq!(store.get_goal("n").unwrap().unwrap().verify_runs, 0);
        assert!(goal_host(&store, "n", r#"{"op":"complete","verification":"nothing to run"}"#).is_ok());
    }

    fn run_auto_turn(store: &ControlStore, sid: &str, calls: &[(&str, Value, &str)]) {
        for (name, args, result) in calls {
            observe_tool(sid, name, args, result);
        }
        after_turn(store, sid, 0, false, false).unwrap();
    }

    #[test]
    fn auto_attempt_line_recorded_without_op_progress() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("auto1", Some(&SessionGoal::new("x"))).unwrap();
        run_auto_turn(
            &store,
            "auto1",
            &[("edit", json!({}), "ok"), ("bash", json!({"command":"cargo test"}), "ok")],
        );
        let g = store.get_goal("auto1").unwrap().unwrap();
        assert_eq!(g.attempt_log.len(), 1);
        assert_eq!(g.attempt_log[0], "auto t2 f0 | files=yes | ver=pass | err=");
        assert!(g.attempt_log[0].len() <= MAX_ATTEMPT_LEN);
    }

    #[test]
    fn repeated_normalized_error_is_a_plateau() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("auto2", Some(&SessionGoal::new("x"))).unwrap();
        for i in 0..PLATEAU_TURNS {
            let err = format!("Error: cannot find /tmp/run{i}/a.rs line {i}");
            run_auto_turn(&store, "auto2", &[("edit", json!({}), "ok"), ("bash", json!({}), &err)]);
        }
        let g = store.get_goal("auto2").unwrap().unwrap();
        assert!(g.plateaued(), "{:?}", g.attempt_log);
        assert!(g.continuation_prompt().contains("Plateau detected"));
    }

    #[test]
    fn a_poisoned_turn_lock_does_not_take_the_agent_loop_down() {
        let _ = std::thread::spawn(|| {
            let _held = turn_obs().lock().unwrap();
            let _retries = error_retries().lock().unwrap();
            panic!("poison both");
        })
        .join();
        assert!(turn_obs().is_poisoned() && error_retries().is_poisoned());
        observe_tool("poisoned-lock-test", "bash", &serde_json::json!({"command": "ls"}), "ok");
        assert_eq!(turn_obs().lock().unwrap_or_else(|e| e.into_inner())["poisoned-lock-test"].tools, 1);
    }

    #[test]
    fn nonzero_bash_exit_is_a_failed_verification() {
        let sid = "nonzero-exit-test";
        observe_tool(sid, "bash", &serde_json::json!({"command": "cargo test"}), "1 failed\n\nExit code: 101");
        observe_tool(sid, "bash", &serde_json::json!({"command": "npm test"}), "ok\n--- Command finished with exit code: 0 ---");
        let map = turn_obs().lock().unwrap_or_else(|e| e.into_inner());
        let obs = &map[sid];
        assert_eq!((obs.tools, obs.failed), (2, 1));
        assert!(obs.verified, "the exit-0 test run still counts as a passing verification");
    }

    #[test]
    fn file_change_plus_passing_test_is_not_a_plateau() {
        let store = ControlStore::memory().unwrap();
        store.set_goal("auto3", Some(&SessionGoal::new("x"))).unwrap();
        for _ in 0..PLATEAU_TURNS {
            run_auto_turn(
                &store,
                "auto3",
                &[("edit", json!({}), "ok"), ("bash", json!({"command":"cargo test"}), "ok")],
            );
        }
        assert!(!store.get_goal("auto3").unwrap().unwrap().plateaued());
    }

    #[test]
    fn due_steer_heartbeat_ignores_follow_up_and_fires_rlm_steer() {
        let store = ControlStore::memory().unwrap();
        let mut follow_up = Heartbeat::new("s1", "watch build", 60);
        follow_up.source = "rlm".into();
        follow_up.last_fired_at_ms = now_ms() - 61_000;
        store.upsert_heartbeat(&follow_up).unwrap();
        assert!(due_steer_heartbeat(&store, "s1").unwrap().is_none());

        let mut steer = Heartbeat::new("s1", "check progress", 60);
        steer.source = "rlm".into();
        steer.delivery_mode = "steer".into();
        steer.last_fired_at_ms = now_ms() - 61_000;
        store.upsert_heartbeat(&steer).unwrap();
        let due = due_steer_heartbeat(&store, "s1").unwrap();
        assert!(matches!(due, Some(Continuation::Heartbeat { .. })));
        // Follow-up heartbeat on the same session is untouched.
        assert!(store.user_heartbeat("s1").unwrap().is_none());
        let stored_follow_up = store
            .list_heartbeats("s1")
            .unwrap()
            .into_iter()
            .find(|h| h.id == follow_up.id)
            .unwrap();
        assert_eq!(stored_follow_up.fire_count, 0);
    }

    #[test]
    fn runner_output_becomes_a_score_vector() {
        use crate::goal_ratchet::*;
        assert_eq!(parse_counts("test result: ok. 5 passed; 1 failed; 0 ignored\ntest result: ok. 2 passed; 0 failed"), Some((7, 1)));
        assert_eq!(parse_counts("=== 2 failed, 5 passed in 0.3s ==="), Some((5, 2)));
        assert_eq!(parse_counts("Tests:       1 failed, 5 passed, 6 total"), Some((5, 1)));
        assert_eq!(score_of(true, "tsc ok"), (1, 0));
        assert_eq!(score_of(false, "boom"), (0, 1));
        assert_eq!(score_of(false, "test result: FAILED. 3 passed; 0 failed"), (3, 1));
    }

    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(o.status.success(), "{args:?}");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn test_run(store: &ControlStore, cwd: &Path, out: &str, exit: i32) {
        let res = format!("{out}\n\nExit code: {exit}");
        observe_tool("rt", "edit", &json!({}), "ok");
        observe_tool("rt", "bash", &json!({"command": "cargo test"}), &res);
        after_turn_in(store, "rt", 0, false, false, Some(cwd)).unwrap();
    }

    /// Inject a throwaway checkpoint store for this test thread (the real one is never used).
    fn test_store(name: &str) -> (std::path::PathBuf, factr_base::checkpoint_store::Store) {
        let base = std::env::temp_dir().join(format!("ratchet-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        crate::goal_ratchet::TEST_STORE.with(|s| *s.borrow_mut() = Some(base.clone()));
        (base.clone(), factr_base::checkpoint_store::Store::at(base))
    }

    #[test]
    fn ratchet_checkpoints_into_the_shared_store_and_regression_points_at_it() {
        let dir = std::env::temp_dir().join(format!("ratchet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let (base, store_cp) = test_store("main");
        git_ok(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "1").unwrap();
        git_ok(&dir, &["add", "a.txt"]);
        git_ok(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
        let head = git_ok(&dir, &["rev-parse", "HEAD"]);
        let store = ControlStore::memory().unwrap();
        store.set_goal("rt", Some(&SessionGoal::new("x"))).unwrap();
        std::fs::write(dir.join("new.txt"), "untracked").unwrap();
        test_run(&store, &dir, "test result: FAILED. 0 passed; 2 failed", 101);
        assert!(store.get_goal("rt").unwrap().unwrap().lineage.is_empty(), "a failing run never commits");
        test_run(&store, &dir, "test result: ok. 3 passed; 0 failed", 0);
        let g = store.get_goal("rt").unwrap().unwrap();
        let r = g.lineage[0].git_ref.clone();
        assert!(r.len() == 40 && r.chars().all(|c| c.is_ascii_hexdigit()), "a store commit hash: {r}");
        let files = store_cp.list(&dir);
        assert_eq!(files[0].hash, r);
        assert!(files[0].message.starts_with("goal checkpoint 1"), "{}", files[0].message);
        assert_eq!(store_cp.pinned(&dir, &crate::goal_ratchet::pin_owner("rt")).len(), 1, "pinned for the goal");
        assert_eq!(git_ok(&dir, &["rev-parse", "HEAD"]), head, "HEAD untouched");
        assert_eq!(git_ok(&dir, &["status", "--porcelain"]), "?? new.txt", "index and worktree untouched");
        assert_eq!(git_ok(&dir, &["for-each-ref", "--format=%(refname)"]).lines().count(), 1, "no ref was written into the user's repo");
        test_run(&store, &dir, "test result: FAILED. 2 passed; 1 failed", 101);
        let p = store.get_goal("rt").unwrap().unwrap().continuation_prompt();
        assert!(p.contains("[Regression]") && p.contains(&format!("diff {r}")) && p.contains(&format!("show {r}:<path>")) && p.contains("--git-dir="), "{p}");
        // no store (checkpoints unavailable) records no-vcs
        crate::goal_ratchet::TEST_STORE.with(|s| *s.borrow_mut() = None);
        let plain = std::env::temp_dir().join(format!("plain-{}", std::process::id()));
        std::fs::create_dir_all(&plain).unwrap();
        store.set_goal("rt", Some(&SessionGoal::new("y"))).unwrap();
        test_run(&store, &plain, "test result: ok. 1 passed; 0 failed", 0);
        assert_eq!(store.get_goal("rt").unwrap().unwrap().lineage[0].git_ref, "no-vcs");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(plain).ok();
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn removed_tests_do_not_freeze_the_ratchet_and_pins_are_released_on_complete_and_clear() {
        let dir = std::env::temp_dir().join(format!("ratchet-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let (base, store_cp) = test_store("prune");
        git_ok(&dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "1").unwrap();
        git_ok(&dir, &["add", "a.txt"]);
        git_ok(&dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);
        let store = ControlStore::memory().unwrap();
        store.set_goal("rt", Some(&SessionGoal::new("x"))).unwrap();
        test_run(&store, &dir, "test result: ok. 7 passed; 0 failed", 0);
        std::fs::write(dir.join("a.txt"), "2").unwrap();
        test_run(&store, &dir, "test result: ok. 5 passed; 0 failed", 0); // tests deleted, none failing
        let g = store.get_goal("rt").unwrap().unwrap();
        assert_eq!(g.lineage.len(), 2, "fewer passes with no failures still checkpoints");
        let owner = crate::goal_ratchet::pin_owner("rt");
        let pins = || store_cp.pinned(&dir, &owner).into_iter().map(|c| c.hash).collect::<Vec<_>>();
        assert_eq!(pins().len(), 2);
        let last = g.lineage[1].git_ref.clone();
        handle_goal_command(&store, "rt", "complete").unwrap();
        assert_eq!(pins(), [last], "completion keeps only the final best checkpoint");
        store.set_goal("rt", Some(&g)).unwrap();
        handle_goal_command(&store, "rt", "clear").unwrap();
        assert!(pins().is_empty());
        crate::goal_ratchet::TEST_STORE.with(|s| *s.borrow_mut() = None);
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn supervisor_is_once_per_plateau_and_rate_limited() {
        let store = ControlStore::memory().unwrap();
        let mut g = SessionGoal::new("port the parser");
        for _ in 0..PLATEAU_TURNS {
            g.record_attempt("auto t1 f1 | files=no | ver=none | err=boom");
        }
        g.turns_used = 3;
        assert!(g.supervisor_due());
        assert!(g.supervisor_request().1.contains("port the parser"));
        store.set_goal("s", Some(&g)).unwrap();
        let p = apply_supervisor(&store, "s", Some("1. rewrite\n2. bisect")).unwrap();
        assert!(p.contains("[Supervisor: alternative directions]") && p.contains("bisect"));
        let after = store.get_goal("s").unwrap().unwrap();
        assert!(!after.supervisor_due(), "same plateau episode");
        assert!(!after.continuation_prompt().contains("Supervisor"), "injected once");
        let mut g2 = after.clone();
        g2.sup_episode = false;
        g2.turns_used = 5;
        assert!(!g2.supervisor_due(), "needs 5 turns since the last call");
        g2.turns_used = 8;
        assert!(g2.supervisor_due());
        // A failed call still consumes the episode and falls back to the text steer.
        store.set_goal("s", Some(&g2)).unwrap();
        assert!(apply_supervisor(&store, "s", None).unwrap().contains("Plateau detected"));
    }
}
