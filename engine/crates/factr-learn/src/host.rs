//! Engine side of the factr-learn REPL: one worker process per session, spawned on
//! first use and reaped when idle, so an unused REPL costs no memory.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Time the code itself may run per cell, and the extra time allowed while a
/// host call (model sub-queries) is in flight. A cell with no host calls gets 20s.
/// Extra host wait granted per wave of `BATCH_CONCURRENCY` prompts in an `llm_query_batch` call.
const BATCH_WAVE_ALLOWANCE: Duration = Duration::from_secs(30);
pub const COMPUTE_TIMEOUT: Duration = Duration::from_secs(20);
const HOST_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const IDLE_REAP_AFTER: Duration = Duration::from_secs(10 * 60);
/// After the interrupt (SIGINT) that ends an over-long cell, how long the worker gets to report
/// the resulting `KeyboardInterrupt` before it is killed. Python raises it within one bytecode
/// instruction, so a second is ample; a worker stuck in native code gets the kill.
const INTERRUPT_GRACE: Duration = Duration::from_secs(2);
const RSS_POLL: Duration = Duration::from_millis(100);
pub const MAX_LOAD_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_QUERY_CHARS: usize = 200_000;
pub const MAX_HOST_CALLS: usize = 16;
pub const BATCH_CONCURRENCY: usize = 8;
pub const MAX_BATCH_PROMPTS: usize = 64;
pub const MAX_BATCH_BYTES: usize = 2_000_000;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
/// Recursive model call used by `llm_query(prompt)`.
pub type LlmQuery = Arc<dyn Fn(String) -> BoxFuture<Result<String>> + Send + Sync>;
/// `refine(op_json)` where `op_json` is `{"op":"run","instructions":...,"global":...}`
/// or `{"op":"status"}`; mirrors factr-learn's `refine.run()`/`refine.status()`. Like
/// the model-callable `refine` tool, this only *schedules* a refinement
/// (applied at turn end, never mid-turn) or reports whether one is pending.
pub type Refine = Arc<dyn Fn(String) -> BoxFuture<Result<String>> + Send + Sync>;
/// Generic host callback (`goal`, `heartbeat`, `spawn_subagent`, `agent_message`).
pub type HostFn = Arc<dyn Fn(String) -> BoxFuture<Result<String>> + Send + Sync>;

/// Optional REPL host hooks beyond `llm_query` / `refine`.
#[derive(Clone)]
pub struct ExtraHostFns {
    pub goal: HostFn,
    pub heartbeat: HostFn,
    pub spawn_subagent: HostFn,
    pub agent_message: HostFn,
    pub websearch: HostFn,
    pub compact: HostFn,
    pub skill: HostFn,
}

impl Default for ExtraHostFns {
    fn default() -> Self {
        fn unavailable(name: &'static str) -> HostFn {
            Arc::new(move |_| {
                let name = name.to_string();
                Box::pin(async move { Err(anyhow!("{name} is not available")) })
            })
        }
        Self {
            goal: unavailable("goal"),
            heartbeat: unavailable("heartbeat"),
            spawn_subagent: unavailable("spawn_subagent"),
            agent_message: unavailable("agent_message"),
            websearch: unavailable("websearch"),
            compact: unavailable("compact"),
            skill: unavailable("skill"),
        }
    }
}

pub struct RunOutput {
    pub stdout: String,
    pub value: Option<String>,
    pub error: Option<String>,
    /// The worker was (re)started for this run, so earlier variables are gone.
    pub fresh_state: bool,
    pub host_calls: usize,
}

struct Worker {
    child: Child,
    pid: u32,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    last_used: Instant,
    _session_tmp: Option<PathBuf>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        if let Some(path) = &self._session_tmp {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

pub struct ReplHost {
    python: PathBuf,
    workers: Mutex<HashMap<String, Arc<Mutex<Option<Worker>>>>>,
}

impl ReplHost {
    /// `python` is the Factr-bundled CPython interpreter.
    pub fn new(python: PathBuf) -> Arc<Self> {
        let host = Arc::new(Self {
            python,
            workers: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&host);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(60));
                loop {
                    tick.tick().await;
                    let Some(host) = weak.upgrade() else { return };
                    host.reap_idle().await;
                }
            });
        }
        host
    }

    async fn reap_idle(&self) {
        let slots: Vec<_> = self.workers.lock().await.values().cloned().collect();
        for slot in slots {
            if let Ok(mut guard) = slot.try_lock() {
                if guard
                    .as_ref()
                    .is_some_and(|w| w.last_used.elapsed() > IDLE_REAP_AFTER)
                {
                    *guard = None; // kill_on_drop ends the process
                }
            }
        }
    }

    /// Number of live worker processes (for tests and diagnostics).
    pub async fn live_workers(&self) -> usize {
        let slots: Vec<_> = self.workers.lock().await.values().cloned().collect();
        let mut n = 0;
        for slot in slots {
            n += usize::from(slot.lock().await.is_some());
        }
        n
    }

    /// Stop and remove the persistent kernel when its owning chat is deleted.
    pub async fn stop_session(&self, session: &str) {
        if let Some(slot) = self.workers.lock().await.remove(session) {
            *slot.lock().await = None;
        }
    }

    #[cfg(target_os = "macos")]
    async fn spawn(&self, workdir: Option<&Path>) -> Result<Worker> {
        let (mut child, session_tmp) = {
            let cwd;
            let project = match workdir {
                Some(path) => path,
                None => {
                    cwd = std::env::current_dir()?;
                    &cwd
                }
            };
            let project = std::fs::canonicalize(project)
                .context("resolving the session working directory")?;
            let session_tmp =
                std::env::temp_dir().join(format!("factr-repl-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&session_tmp)?;
            let session_tmp = std::fs::canonicalize(session_tmp)?;
            let skills = skills_dir()?;
            // Launch by the path as given, not its canonical target: a virtualenv interpreter is a
            // symlink and finds its packages through `pyvenv.cfg` next to that path.
            // Only the directory is resolved (the sandbox matches real paths, and /var is /private/var).
            let given = std::path::absolute(&self.python).context("resolving the REPL interpreter")?;
            let python = match (given.parent().map(std::fs::canonicalize), given.file_name()) {
                (Some(Ok(dir)), Some(name)) => dir.join(name),
                _ => given,
            };
            let real = std::fs::canonicalize(&python).context("resolving the REPL interpreter")?;
            let runtime = python_runtime_root(&real)?;
            let probe = python.clone();
            let imports = tokio::task::spawn_blocking(move || factr_base::python_env::read_roots(&probe))
                .await
                .context("probing the interpreter's import paths")?;
            let profile =
                sandbox_profile(&python, &real, &runtime, &project, &session_tmp, &skills, &imports);
            let child = Command::new("/usr/bin/sandbox-exec")
                .args(["-p", &profile])
                .arg(&python)
                // No `-S`: the interpreter's own site-packages must load (one environment with bash).
                // `-E` ignores PYTHON* variables; the worker drops the working directory from sys.path.
                .args(["-E", "-u", "-c", crate::worker::PYTHON_WORKER])
                .arg(&project)
                .arg(&skills)
                // Start inside the project directory (inside the sandbox's allowed set) so
                // `os.getcwd()` and relative `open()` work; inheriting the engine's cwd left
                // the worker in a directory the profile forbids.
                .current_dir(&project)
                // The worker receives no model credentials or arbitrary environment.
                .env_clear()
                .env("PYTHONDONTWRITEBYTECODE", "1")
                .env("TMPDIR", &session_tmp)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .context("starting the sandboxed CPython worker")?;
            (child, session_tmp)
        };
        let stdin = child.stdin.take().context("worker stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("worker stdout")?).lines();
        let ready = tokio::time::timeout(Duration::from_secs(10), stdout.next_line())
            .await
            .context("REPL worker did not start")??
            .context("REPL worker exited during startup")?;
        let ready: Value =
            serde_json::from_str(&ready).context("REPL worker sent malformed startup data")?;
        if ready["op"] != "ready" {
            bail!("REPL worker sent an unexpected greeting");
        }
        let pid = ready["pid"]
            .as_u64()
            .unwrap_or_else(|| child.id().unwrap_or_default() as u64) as u32;
        Ok(Worker {
            child,
            pid,
            stdin,
            stdout,
            last_used: Instant::now(),
            _session_tmp: Some(session_tmp),
        })
    }

    #[cfg(not(target_os = "macos"))]
    async fn spawn(&self, workdir: Option<&Path>) -> Result<Worker> {
        let cwd;
        let project = match workdir {
            Some(path) => path,
            None => {
                cwd = std::env::current_dir()?;
                &cwd
            }
        };
        let project = std::fs::canonicalize(project)?;
        let session_tmp =
            std::env::temp_dir().join(format!("factr-repl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&session_tmp)?;
        let skills = skills_dir()?;
        let mut child = Command::new(&self.python)
            .args(["-E", "-u", "-c", crate::worker::PYTHON_WORKER])
            .arg(&project)
            .arg(&skills)
            .current_dir(&project)
            .env_clear()
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("TMPDIR", &session_tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("starting the approved CPython worker")?;
        let stdin = child.stdin.take().context("worker stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("worker stdout")?).lines();
        let ready = tokio::time::timeout(Duration::from_secs(10), stdout.next_line())
            .await
            .context("REPL worker did not start")??
            .context("REPL worker exited during startup")?;
        let ready: Value = serde_json::from_str(&ready)?;
        if ready["op"] != "ready" {
            bail!("REPL worker sent an unexpected greeting");
        }
        let pid = ready["pid"]
            .as_u64()
            .unwrap_or_else(|| child.id().unwrap_or_default() as u64) as u32;
        Ok(Worker {
            child,
            pid,
            stdin,
            stdout,
            last_used: Instant::now(),
            _session_tmp: Some(session_tmp),
        })
    }

    /// Run one cell. `memory_limit` is the worker's resident-memory cap in bytes; a worker over it is
    /// killed (the host stays usable and the next cell starts a fresh worker).
    pub async fn run(
        &self,
        session: &str,
        code: &str,
        workdir: Option<&Path>,
        llm_query: LlmQuery,
        refine: Refine,
        extra: ExtraHostFns,
        memory_limit: u64,
    ) -> Result<RunOutput> {
        let slot = self
            .workers
            .lock()
            .await
            .entry(session.to_string())
            .or_default()
            .clone();
        let mut guard = slot.lock().await;
        let fresh_state = guard.is_none();
        if guard.is_none() {
            *guard = Some(self.spawn(workdir).await?);
        }
        let worker = guard.as_mut().expect("worker present");
        let worker_pid = worker.pid;
        let result = tokio::select! {
            result = drive(worker, code, workdir, &llm_query, &refine, &extra) => result,
            _ = wait_for_rss_limit(worker_pid, memory_limit) => {
                let _ = worker.child.start_kill();
                *guard = None;
                bail!(over_limit(memory_limit))
            }
        };
        if result.is_ok() && over_rss(worker_pid, memory_limit) {
            let _ = worker.child.start_kill();
            *guard = None;
            bail!(over_limit(memory_limit))
        }
        match result {
            Ok((stdout, value, error, host_calls)) => {
                worker.last_used = Instant::now();
                Ok(RunOutput {
                    stdout,
                    value,
                    error,
                    fresh_state,
                    host_calls,
                })
            }
            Err(err) => {
                *guard = None;
                Err(err.context("the REPL worker stopped and was reset; variables were lost"))
            }
        }
    }
}

fn over_rss(pid: u32, limit: u64) -> bool {
    factr_base::platform::process_rss_bytes(pid).is_some_and(|rss| rss > limit)
}

async fn wait_for_rss_limit(pid: u32, limit: u64) {
    while !over_rss(pid, limit) {
        tokio::time::sleep(RSS_POLL).await;
    }
}

fn over_limit(limit: u64) -> String {
    let gib = (limit as f64 / (1u64 << 30) as f64 * 100.0).round() / 100.0;
    format!("the REPL exceeded {gib} GiB resident memory and was restarted (variables were lost); stream or chunk the data")
}

#[cfg(target_os = "macos")]
fn sbpl_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// The skills directory the worker may import from. With learning off (`FACTR_LEARNING_ENABLED=0`) it
/// is neither created nor seeded, so the REPL leaves no `skills/` behind; the worker tolerates a
/// missing directory.
fn skills_dir() -> Result<PathBuf> {
    let skills = factr_storage::factr_dir()?.join("skills");
    if std::env::var("FACTR_LEARNING_ENABLED").is_ok_and(|v| v == "0") {
        return Ok(std::fs::canonicalize(&skills).unwrap_or(skills));
    }
    std::fs::create_dir_all(&skills)?;
    crate::bundled_skills::install(&skills)?;
    Ok(std::fs::canonicalize(skills)?)
}

#[cfg(target_os = "macos")]
fn sandbox_profile(
    python: &Path,
    real_python: &Path,
    runtime: &Path,
    project: &Path,
    temp: &Path,
    skills: &Path,
    imports: &[PathBuf],
) -> String {
    let mut reads = vec![runtime, project, skills];
    reads.extend(imports.iter().map(PathBuf::as_path));
    let reads: String = reads.iter().map(|p| format!(" (subpath \"{}\")", sbpl_path(p))).collect();
    format!(
        "(version 1)\n(deny default)\n(import \"system.sb\")\n(deny network*)\n(deny file-write*)\n(allow file-read-metadata)\n(deny file-read* (subpath \"/etc\"))\n(deny file-read* (subpath \"/private/etc\"))\n(deny file-read* (subpath \"/Volumes\"))\n(allow process-exec (literal \"{}\") (literal \"{}\") (subpath \"{}\"))\n(allow file-read*{} (subpath \"/System/Library\") (subpath \"/usr/lib\") (subpath \"/Library/Developer/CommandLineTools\"))\n(allow file-write* (subpath \"{}\") (subpath \"{}\"))\n",
        sbpl_path(python),
        sbpl_path(real_python),
        sbpl_path(runtime),
        reads,
        sbpl_path(project),
        sbpl_path(temp)
    )
}

/// Whether the REPL can be confined without an explicitly named interpreter: only where the macOS
/// sandbox exists. On Linux the worker would run with `bash`'s privileges plus nothing less (full
/// network and filesystem, no confinement), so the REPL is not enabled there on its own; naming an
/// interpreter in `FACTR_REPL_PYTHON` is the explicit opt-in.
pub fn sandbox_available() -> bool {
    cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file()
}

fn python_runtime_root(python: &Path) -> Result<PathBuf> {
    if let Ok(executable) = std::fs::canonicalize(python) {
        if let Some(root) = executable.parent().and_then(Path::parent) {
            return Ok(root.to_path_buf());
        }
    }
    let venv_root = python
        .parent()
        .and_then(Path::parent)
        .context("resolving bundled Python runtime")?;
    let config = std::fs::read_to_string(venv_root.join("pyvenv.cfg"));
    if let Ok(config) = config {
        if let Some(home) = config
            .lines()
            .find_map(|line| line.strip_prefix("home = ").map(PathBuf::from))
        {
            if let Some(root) = home.parent() {
                return Ok(root.to_path_buf());
            }
        }
    }
    Ok(venv_root.to_path_buf())
}

async fn send(worker: &mut Worker, value: Value) -> Result<()> {
    worker
        .stdin
        .write_all(format!("{value}\n").as_bytes())
        .await?;
    worker.stdin.flush().await?;
    Ok(())
}

/// Ask the REPL worker to stop the running cell (SIGINT). Windows has no per-process SIGINT: the cell
/// is left to run into the grace period, after which the worker is killed.
fn interrupt_worker(pid: u32) {
    #[cfg(unix)]
    // SAFETY: a plain signal to our own child process.
    unsafe {
        libc::kill(pid as i32, libc::SIGINT);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

async fn drive(
    worker: &mut Worker,
    code: &str,
    workdir: Option<&Path>,
    llm_query: &LlmQuery,
    refine: &Refine,
    extra: &ExtraHostFns,
) -> Result<(String, Option<String>, Option<String>, usize)> {
    send(worker, json!({"op": "run", "code": code})).await?;
    let mut host_calls = 0;
    let mut compute_left = COMPUTE_TIMEOUT;
    let mut host_left = HOST_WAIT_TIMEOUT;
    let mut interrupted = false;
    loop {
        let waited = Instant::now();
        let line = match tokio::time::timeout(compute_left, worker.stdout.next_line()).await {
            Ok(line) => line?.ok_or_else(|| anyhow!("REPL worker exited"))?,
            // Over time: interrupt first, so the cell ends as an ordinary KeyboardInterrupt error and
            // the session's variables survive; only a worker that ignores it is killed.
            Err(_) if !interrupted => {
                interrupted = true;
                compute_left = INTERRUPT_GRACE;
                interrupt_worker(worker.pid);
                continue;
            }
            Err(_) => bail!(
                "the REPL cell exceeded {}s of compute and did not stop when interrupted",
                COMPUTE_TIMEOUT.as_secs()
            ),
        };
        compute_left = compute_left.saturating_sub(waited.elapsed());
        let msg: Value = serde_json::from_str(&line).context("REPL worker protocol error")?;
        match msg["op"].as_str() {
            Some("done") => {
                let text = |k: &str| msg[k].as_str().map(str::to_owned);
                let error = match (text("error"), interrupted) {
                    (Some(error), true) => Some(format!(
                        "the cell exceeded {}s of compute and was interrupted (variables are kept)\n{error}",
                        COMPUTE_TIMEOUT.as_secs()
                    )),
                    (error, _) => error,
                };
                return Ok((
                    text("stdout").unwrap_or_default(),
                    text("value"),
                    error,
                    host_calls,
                ));
            }
            Some("call") => {
                host_calls += 1;
                if host_calls > MAX_HOST_CALLS {
                    send(
                        worker,
                        json!({"op":"reply", "error":"host call budget (16) exhausted"}),
                    )
                    .await?;
                    continue;
                }
                let arg = msg["args"][0].as_str().unwrap_or_default().to_string();
                let started = Instant::now();
                let allowance = if msg["fn"].as_str() == Some("llm_query_batch") { batch_allowance(&arg) } else { Duration::ZERO };
                let call = async { match msg["fn"].as_str() {
                    Some("llm_query") => match within_query_cap(arg.clone()) {
                        Ok(prompt) => llm_query(prompt).await,
                        Err(err) => Err(err),
                    },
                    Some("llm_query_batch") => llm_query_batch(&llm_query, &arg).await,
                    Some("load_path") => load_path(workdir, &arg).await,
                    Some("load") => load(workdir, &arg).await,
                    Some("refine") => refine(truncate(arg, MAX_QUERY_CHARS)).await,
                    Some("goal") => (extra.goal)(truncate(arg, MAX_QUERY_CHARS)).await,
                    Some("heartbeat") => (extra.heartbeat)(truncate(arg, MAX_QUERY_CHARS)).await,
                    Some("spawn_subagent") => {
                        (extra.spawn_subagent)(truncate(arg, MAX_QUERY_CHARS)).await
                    }
                    Some("agent_message") => {
                        (extra.agent_message)(truncate(arg, MAX_QUERY_CHARS)).await
                    }
                    Some("websearch") => (extra.websearch)(truncate(arg, MAX_QUERY_CHARS)).await,
                    Some("compact") => (extra.compact)(truncate(arg, MAX_QUERY_CHARS)).await,
                    Some("skill") => (extra.skill)(truncate(arg, MAX_QUERY_CHARS)).await,
                    _ => Err(anyhow!("unknown host function")),
                } };
                // A batch earns extra wait per wave of concurrent sub-queries.
                host_left += allowance;
                let reply = match tokio::time::timeout(host_left, call).await {
                    Ok(reply) => reply,
                    // Like the call budget: the cell gets an error and its worker and variables survive.
                    Err(_) => {
                        host_left = Duration::ZERO;
                        Err(anyhow!(
                            "host call time budget exhausted: the cell may wait {}s in total on model calls (more for a large batch); split the work across cells",
                            HOST_WAIT_TIMEOUT.as_secs()
                        ))
                    }
                };
                host_left = host_left.saturating_sub(started.elapsed());
                let reply = match reply {
                    Ok(value) => json!({"op": "reply", "value": value}),
                    Err(err) => json!({"op": "reply", "error": format!("{err:#}")}),
                };
                send(worker, reply).await?;
            }
            _ => bail!("REPL worker protocol error"),
        }
    }
}

fn batch_allowance(prompts_json: &str) -> Duration {
    let n = serde_json::from_str::<Vec<Value>>(prompts_json).map_or(0, |v| v.len().min(MAX_BATCH_PROMPTS));
    BATCH_WAVE_ALLOWANCE * n.div_ceil(BATCH_CONCURRENCY) as u32
}

/// A prompt over the cap is an explicit error for the caller, never silently cut.
fn within_query_cap(prompt: String) -> Result<String> {
    let chars = prompt.chars().count();
    if chars > MAX_QUERY_CHARS {
        bail!("llm_query: prompt is {chars} characters, the limit is {MAX_QUERY_CHARS}; split the input into smaller slices");
    }
    Ok(prompt)
}

fn truncate(text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}\n[truncated]", &text[..cut]),
        None => text,
    }
}

/// Resolve a workspace file for `load(path)`: confined to the session's working
/// directory after resolving symlinks. Returns the real path and its size.
async fn resolve_workspace_file(workdir: Option<&Path>, path: &str) -> Result<(PathBuf, u64)> {
    let root = workdir.context("load() needs a session working directory")?;
    let root = tokio::fs::canonicalize(root)
        .await
        .context("resolving the working directory")?;
    let requested = Path::new(path);
    let joined = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let resolved = tokio::fs::canonicalize(&joined)
        .await
        .with_context(|| format!("{path}: not found"))?;
    if !resolved.starts_with(&root) {
        bail!("{path}: outside the working directory");
    }
    let meta = tokio::fs::metadata(&resolved).await?;
    if !meta.is_file() {
        bail!("{path}: not a file");
    }
    Ok((resolved, meta.len()))
}

/// Host side of `load(path, start, length)`: validates the path and returns
/// `{"path","size"}` so the sandboxed worker reads only the slice it asked for
/// from disk (the file never rides the JSON pipe or sits whole in its memory).
pub async fn load_path(workdir: Option<&Path>, path: &str) -> Result<String> {
    let (resolved, size) = resolve_workspace_file(workdir, path).await?;
    Ok(json!({"path": resolved, "size": size}).to_string())
}

/// Whole-file read, size-capped (host-side convenience; the worker slices itself).
pub async fn load(workdir: Option<&Path>, path: &str) -> Result<String> {
    let (resolved, size) = resolve_workspace_file(workdir, path).await?;
    if size > MAX_LOAD_BYTES {
        bail!("{path}: larger than {} MB", MAX_LOAD_BYTES / (1024 * 1024));
    }
    let bytes = tokio::fs::read(&resolved).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Host side of `llm_query_batch`: `prompts_json` is a JSON array of strings.
/// Runs up to 8 sub-queries at once and returns a JSON array of replies in
/// order; a failed item becomes an error string, not a failure of the call.
pub async fn llm_query_batch(llm_query: &LlmQuery, prompts_json: &str) -> Result<String> {
    let prompts: Vec<String> = serde_json::from_str(prompts_json)
        .context("llm_query_batch expects a JSON list of strings")?;
    if prompts.is_empty() {
        bail!("llm_query_batch: the list is empty; pass one prompt per record, each containing that record's text");
    }
    if prompts.len() > MAX_BATCH_PROMPTS {
        bail!(
            "llm_query_batch: {} prompts, the limit is {MAX_BATCH_PROMPTS}",
            prompts.len()
        );
    }
    let total: usize = prompts.iter().map(String::len).sum();
    if total > MAX_BATCH_BYTES {
        bail!("llm_query_batch: {total} bytes of input, the limit is {MAX_BATCH_BYTES}");
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(BATCH_CONCURRENCY));
    let mut jobs = tokio::task::JoinSet::new();
    for (index, prompt) in prompts.into_iter().enumerate() {
        let call = within_query_cap(prompt).map(|p| llm_query(p));
        let gate = gate.clone();
        jobs.spawn(async move {
            let _slot = gate.acquire_owned().await;
            (index, match call {
                Ok(call) => call.await,
                Err(err) => Err(err),
            })
        });
    }
    let mut replies = vec![String::new(); jobs.len()];
    while let Some(done) = jobs.join_next().await {
        let (index, result) = done.context("llm_query_batch task failed")?;
        replies[index] = match result {
            Ok(text) => text,
            Err(err) => format!("Error: {err:#}"),
        };
    }
    Ok(serde_json::to_string(&replies)?)
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting(live: Arc<AtomicUsize>, peak: Arc<AtomicUsize>) -> LlmQuery {
        Arc::new(move |prompt: String| {
            let (live, peak) = (live.clone(), peak.clone());
            Box::pin(async move {
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // Earlier prompts finish later, so ordering is not completion order.
                let n: u64 = prompt.strip_prefix('p').and_then(|n| n.parse().ok()).unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(30 - n.min(25))).await;
                live.fetch_sub(1, Ordering::SeqCst);
                if prompt == "bad" {
                    return Err(anyhow!("boom"));
                }
                Ok(format!("{}:{}", prompt, prompt.len()))
            })
        })
    }

    #[test]
    fn batch_wait_scales_with_waves() {
        assert_eq!(batch_allowance("not json"), Duration::ZERO);
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 8])), BATCH_WAVE_ALLOWANCE);
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 64])), BATCH_WAVE_ALLOWANCE * 8);
    }

    fn json_list(items: &[String]) -> String {
        serde_json::to_string(items).unwrap()
    }

    #[tokio::test]
    async fn runs_eight_at_a_time_in_order_with_error_strings() {
        let (live, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let mut prompts: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
        prompts[3] = "bad".into();
        let out = llm_query_batch(&counting(live, peak.clone()), &json_list(&prompts)).await.unwrap();
        let out: Vec<String> = serde_json::from_str(&out).unwrap();
        assert_eq!(out.len(), 20);
        assert_eq!(out[0], "p0:2");
        assert_eq!(out[3], "Error: boom");
        assert_eq!(out[19], "p19:3");
        assert_eq!(peak.load(Ordering::SeqCst), 8, "bounded concurrency of exactly 8");
    }

    #[tokio::test]
    async fn enforces_prompt_count_total_bytes_and_per_item_cap() {
        let (live, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let q = counting(live, peak);
        let many = json_list(&vec!["x".to_string(); 65]);
        assert!(llm_query_batch(&q, &many).await.unwrap_err().to_string().contains("limit is 64"));
        let big = json_list(&vec!["x".repeat(190_000); 11]);
        assert!(llm_query_batch(&q, &big).await.unwrap_err().to_string().contains("bytes of input"));
        let one = json_list(&["y".repeat(300_000)]);
        let out: Vec<String> = serde_json::from_str(&llm_query_batch(&q, &one).await.unwrap()).unwrap();
        assert!(out[0].starts_with("Error: llm_query: prompt is 300000 characters, the limit is 200000"), "{}", out[0]);
        let edge = json_list(&["y".repeat(MAX_QUERY_CHARS)]);
        let out: Vec<String> = serde_json::from_str(&llm_query_batch(&q, &edge).await.unwrap()).unwrap();
        assert!(out[0].starts_with("yyy"), "a prompt at the cap goes through whole");
        assert!(within_query_cap("z".repeat(MAX_QUERY_CHARS + 1)).is_err());
        assert_eq!(within_query_cap("ok".into()).unwrap(), "ok");
        assert!(llm_query_batch(&q, "not json").await.is_err());
        let empty = llm_query_batch(&q, "[]").await.unwrap_err().to_string();
        assert!(empty.contains("list is empty"), "{empty}");
    }
}
