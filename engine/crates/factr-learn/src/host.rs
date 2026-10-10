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
/// Default number of sub-queries a batch runs at once; `FACTR_BATCH_CONCURRENCY` overrides it
/// (see [`batch_concurrency`]).
pub const BATCH_CONCURRENCY: usize = 8;
pub const MAX_BATCH_PROMPTS: usize = 64;
pub const MAX_BATCH_BYTES: usize = 2_000_000;
/// Longest backoff wait the worker may ask the engine for.
const MAX_SLEEP_MS: u64 = 8_000;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
/// Recursive model call used by `llm_query(prompt)`.
pub type LlmQuery = Arc<dyn Fn(String) -> BoxFuture<Result<String>> + Send + Sync>;
/// One sub-model reply with what the provider reported about the call. Used by `classify` only, to
/// log per-chunk cost; a field the provider does not report is `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubReply {
    pub text: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub latency_ms: u64,
    /// The reasoning effort the call ran with (`None`: the model default or the inherited effort).
    pub effort: Option<String>,
    /// The effort the call asked for (differs from `effort` when the API refused it and the call fell back).
    pub requested_effort: Option<String>,
    /// When the API refused the requested effort first: how long that refused request took (ms).
    pub refused_ms: Option<u64>,
}
/// [`LlmQuery`] that also reports usage and latency.
pub type LlmQueryMeta = Arc<dyn Fn(String) -> BoxFuture<Result<SubReply>> + Send + Sync>;
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
    /// Sub-model call with usage (for the `classify` log); `None` falls back to the plain `llm_query`.
    pub llm_query_meta: Option<LlmQueryMeta>,
    /// The reasoning effort sub-calls run with, part of `classify`'s cache key (empty: inherited).
    pub sub_effort: String,
    /// The effort the user configured for `classify` (empty: inherit). Differs from `sub_effort` when the
    /// model refused it; the classify log carries both.
    pub sub_requested: String,
    /// The provider/model id sub-calls run on, part of `classify`'s cache key (empty: unknown).
    pub sub_model: String,
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
            llm_query_meta: None,
            sub_effort: String::new(),
            sub_requested: String::new(),
            sub_model: String::new(),
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
    /// Asked at each worker start for the interpreter to use now (the session environment may have
    /// been created after the host was); `python` is the fallback.
    resolver: Option<fn() -> Option<PathBuf>>,
    workers: Mutex<HashMap<String, Arc<Mutex<Option<Worker>>>>>,
}

impl ReplHost {
    /// `python` is the Factr-bundled CPython interpreter.
    pub fn new(python: PathBuf) -> Arc<Self> {
        Self::new_resolving(python, None)
    }

    /// Like [`ReplHost::new`], but each new worker runs on `resolver()` when it returns a path.
    pub fn new_resolving(python: PathBuf, resolver: Option<fn() -> Option<PathBuf>>) -> Arc<Self> {
        let host = Arc::new(Self {
            python,
            resolver,
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
        let interpreter = self.resolver.and_then(|resolve| resolve()).unwrap_or_else(|| self.python.clone());
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
            let given = std::path::absolute(&interpreter).context("resolving the REPL interpreter")?;
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
        let interpreter = self.resolver.and_then(|resolve| resolve()).unwrap_or_else(|| self.python.clone());
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
        let mut child = Command::new(&interpreter)
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
        let mut cfg = classify_cfg(&|key| std::env::var(key).ok(), &extra.sub_effort, &extra.sub_model);
        cfg["requested_effort"] = json!(extra.sub_requested);
        let stamp = log_stamp(session, next_cell(session), std::env::var("FACTR_RUN_ID").ok().as_deref());
        let result = tokio::select! {
            result = drive(worker, code, workdir, &llm_query, &refine, &extra, &cfg, &stamp) => result,
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
    cfg: &Value,
    stamp: &serde_json::Map<String, Value>,
) -> Result<(String, Option<String>, Option<String>, usize)> {
    send(worker, json!({"op": "run", "code": code, "cfg": cfg})).await?;
    let conc = cfg["concurrency"].as_u64().map_or(BATCH_CONCURRENCY, |n| n as usize);
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
            // The classify log is bookkeeping: it never spends the cell's host-call budget.
            Some("call") if msg["fn"].as_str() == Some("classify_log") => {
                let reply = match classify_log(msg["args"][0].as_str().unwrap_or_default(), stamp) {
                    Ok(()) => json!({"op": "reply", "value": ""}),
                    Err(err) => json!({"op": "reply", "error": format!("{err:#}")}),
                };
                send(worker, reply).await?;
            }
            // A backoff wait the worker asks the engine to spend: it must not count as the cell's compute
            // (the clock only runs while the engine waits on the worker) or its host calls.
            // Bounded by what is left of the cell's model-call wait, so a loop of sleeps cannot hold the cell.
            Some("call") if msg["fn"].as_str() == Some("sleep_ms") => {
                let ms = msg["args"][0].as_str().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let ms = sleep_allowed(ms, host_left);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                host_left = host_left.saturating_sub(Duration::from_millis(ms));
                send(worker, json!({"op": "reply", "value": ""})).await?;
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
                let is_batch = matches!(msg["fn"].as_str(), Some("llm_query_batch" | "llm_query_batch_meta"));
                let allowance = if is_batch { batch_allowance(&arg, conc) } else { Duration::ZERO };
                let call = async { match msg["fn"].as_str() {
                    Some("llm_query") => match within_query_cap(arg.clone()) {
                        Ok(prompt) => llm_query(prompt).await,
                        Err(err) => Err(err),
                    },
                    Some("llm_query_batch") => llm_query_batch_with(llm_query, &arg, conc).await,
                    Some("llm_query_batch_meta") => {
                        let meta = extra.llm_query_meta.clone().unwrap_or_else(|| meta_from_plain(llm_query.clone()));
                        llm_query_batch_meta(&meta, &arg, conc).await
                    }
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

/// How long a `sleep_ms` request may wait: at most [`MAX_SLEEP_MS`] and never more than the cell's
/// remaining model-call wait.
fn sleep_allowed(ms: u64, host_left: Duration) -> u64 {
    ms.min(MAX_SLEEP_MS).min(host_left.as_millis().min(u128::from(u64::MAX)) as u64)
}

/// Extra wait a batch earns: one wave allowance per 8 prompts (or per `concurrency`, if lower). Raising the
/// concurrency never shortens it: a throttled wide batch is as slow as a narrow one, and a timeout drops every
/// call in flight.
fn batch_allowance(prompts_json: &str, concurrency: usize) -> Duration {
    let n = serde_json::from_str::<Vec<Value>>(prompts_json).map_or(0, |v| v.len().min(MAX_BATCH_PROMPTS));
    BATCH_WAVE_ALLOWANCE * n.div_ceil(concurrency.clamp(1, BATCH_CONCURRENCY)) as u32
}

/// This session's next cell number (1, 2, ...), counted by the engine: it survives worker restarts.
fn next_cell(session: &str) -> u64 {
    static CELLS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, u64>>> = std::sync::OnceLock::new();
    let mut cells = CELLS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    let n = cells.entry(session.to_string()).or_insert(0);
    *n += 1;
    *n
}

/// Fields the engine adds to every classify log row: `session`, `cell` and, when `FACTR_RUN_ID` is set,
/// `run_id`. With them, `call_id`, `chunk` and `classify_call` (which restart in a new worker) are unique.
fn log_stamp(session: &str, cell: u64, run_id: Option<&str>) -> serde_json::Map<String, Value> {
    let mut stamp = serde_json::Map::new();
    stamp.insert("session".into(), json!(session.chars().take(LOG_STRING_MAX).collect::<String>()));
    stamp.insert("cell".into(), json!(cell));
    if let Some(run) = run_id.map(str::trim).filter(|r| !r.is_empty()) {
        stamp.insert("run_id".into(), json!(run.chars().take(LOG_STRING_MAX).collect::<String>()));
    }
    stamp
}

/// How many sub-queries a batch runs at once: `FACTR_BATCH_CONCURRENCY` (1 to 64), else 8. The 0.0.3
/// value (8) under `FACTR_COST_LEGACY=1`.
pub fn batch_concurrency() -> usize {
    concurrency_from(&|key| std::env::var(key).ok())
}

fn concurrency_from(env: &dyn Fn(&str) -> Option<String>) -> usize {
    if legacy_from(env) {
        return BATCH_CONCURRENCY;
    }
    env("FACTR_BATCH_CONCURRENCY")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| (1..=MAX_BATCH_PROMPTS).contains(n))
        .unwrap_or(BATCH_CONCURRENCY)
}

/// `FACTR_COST_LEGACY=1` in the process environment.
pub fn cost_legacy() -> bool {
    legacy_from(&|key| std::env::var(key).ok())
}

/// Reasoning efforts a sub-call may be pinned to. `minimal` is not one: the sub-model's API rejects it.
pub const SUB_EFFORTS: [&str; 5] = ["none", "low", "medium", "high", "xhigh"];

/// The reasoning effort `classify` sub-calls run with. `env` (`FACTR_REPL_SUB_EFFORT`) beats `configured`
/// (`agents.repl_sub_effort`). `Ok(None)` means "inherit the main agent's effort", the default: the effort is
/// the user's, never hard-coded here. A lower effort is an explicit opt-in. An unknown value is an error naming
/// the allowed ones; the caller falls back to the main effort.
pub fn resolve_sub_effort(configured: Option<&str>, env: Option<&str>, legacy: bool) -> Result<Option<String>, String> {
    resolve_effort(configured, env, legacy, None)
}

/// Effort of `llm_query` / `llm_query_batch` (reading, summarising, extraction): `FACTR_REPL_QUERY_EFFORT`,
/// else `agents.repl_query_effort`, else the main agent's effort.
pub fn resolve_query_effort(configured: Option<&str>, env: Option<&str>, legacy: bool) -> Result<Option<String>, String> {
    resolve_effort(configured, env, legacy, None)
}

fn resolve_effort(configured: Option<&str>, env: Option<&str>, legacy: bool, default: Option<&str>) -> Result<Option<String>, String> {
    if legacy {
        return Ok(None);
    }
    let pick = [env, configured].into_iter().flatten().map(str::trim).find(|v| !v.is_empty());
    let Some(value) = pick else { return Ok(default.map(str::to_string)) };
    let value = value.to_ascii_lowercase();
    if value == "inherit" {
        return Ok(None);
    }
    if SUB_EFFORTS.contains(&value.as_str()) {
        return Ok(Some(value));
    }
    Err(format!(
        "repl sub-call effort '{value}' is not supported (use {} or inherit); using the main agent's effort",
        SUB_EFFORTS.join("|")
    ))
}

/// `FACTR_COST_LEGACY=1`: the 0.0.3 cost behaviour, for A/B runs.
pub fn legacy_from(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env("FACTR_COST_LEGACY").is_some_and(|v| v.trim() == "1")
}

fn num_env(env: &dyn Fn(&str) -> Option<String>, key: &str, default: u64, min: u64, max: u64) -> u64 {
    env(key).and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| (min..=max).contains(n)).unwrap_or(default)
}

fn opt_num_env(env: &dyn Fn(&str) -> Option<String>, key: &str, min: u64, max: u64) -> Option<u64> {
    env(key).and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| (min..=max).contains(n))
}

/// The `classify` settings the worker gets with every cell (it runs without the engine's
/// environment). Defaults are the 0.0.4 behaviour; `FACTR_COST_LEGACY=1` selects the 0.0.3 one.
/// `effort` and `model` are the effective sub-call effort and model, part of the result cache key.
pub fn classify_cfg(env: &dyn Fn(&str) -> Option<String>, effort: &str, model: &str) -> Value {
    json!({
        "legacy": legacy_from(env),
        "format": if env("FACTR_CLASSIFY_FORMAT").is_some_and(|v| v.trim().eq_ignore_ascii_case("codes")) { "codes" } else { "json" },
        // null: not set by the user, the worker picks 80/48000 or 40/24000 from the data (see python_worker.py).
        "chunk_items": opt_num_env(env, "FACTR_CLASSIFY_CHUNK_ITEMS", 1, 500),
        "chunk_chars": opt_num_env(env, "FACTR_CLASSIFY_CHUNK_CHARS", 500, 150_000),
        "dedupe": env("FACTR_CLASSIFY_DEDUPE").is_some_and(|v| v.trim() == "1"),
        "log": env("FACTR_CLASSIFY_LOG").is_some_and(|v| !v.trim().is_empty()),
        "concurrency": concurrency_from(env),
        "effort": effort,
        "model": model,
    })
}

/// Which keys a `classify` log row may carry, by `type`: nothing else is written. `call` is one sub-call
/// attempt, `occ` one input occurrence with its final label (null if the job failed before labelling it), `job`
/// one per `classify` with its outcome (the names a record-level comparison reads).
const LOG_KEYS: [(&str, &[&str]); 3] = [
    ("call", &["type", "call_id", "classify_call", "pass", "wave", "attempt", "chunk_size", "records_count", "input_tokens", "output_tokens", "reasoning_tokens", "cached_tokens", "latency_ms", "ts_start_ms", "ts_end_ms", "effort", "requested_effort", "effort_fallback", "format", "votes", "model", "legacy", "dedupe", "chunk_items", "chunk_chars", "chunk_source", "concurrency", "prompt_chars", "reply_chars", "error", "refusal", "transport_error", "validation_failure", "results"]),
    ("occ", &["type", "classify_call", "occ", "h", "label", "effort", "truncated", "cached", "deduped", "chunk", "pos"]),
    ("job", &["type", "classify_call", "status", "error", "records", "distinct", "cached", "to_label", "unlabelled", "votes", "format", "effort", "requested_effort", "effort_fallback", "model", "legacy", "dedupe", "chunk_items", "chunk_chars", "chunk_source", "concurrency"]),
];
/// Keys the engine itself adds to every row (see [`log_stamp`]); a row may not carry them on its own.
const STAMP_KEYS: [&str; 3] = ["session", "cell", "run_id"];
const LOG_ROW_MAX: usize = 262_144;
const LOG_STRING_MAX: usize = 1_000;

fn is_hash16(value: &Value) -> bool {
    value.as_str().is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// One log row, re-serialised from only the keys its event allows with plain values (numbers, booleans,
/// null, short strings; hashes must be 16 hex; `results` is a list of `[hash, label]`), or `None`.
fn clean_log_row(line: &str, stamp: &serde_json::Map<String, Value>) -> Option<String> {
    if line.len() > LOG_ROW_MAX {
        return None;
    }
    let row: Value = serde_json::from_str(line).ok()?;
    let object = row.as_object()?;
    let allowed = LOG_KEYS.iter().find(|(kind, _)| object.get("type").and_then(Value::as_str) == Some(*kind))?.1;
    let mut clean = serde_json::Map::new();
    for (key, value) in object {
        if !allowed.contains(&key.as_str()) {
            continue;
        }
        let ok = match (key.as_str(), value) {
            ("h", v) => is_hash16(v),
            ("results", Value::Array(rows)) => rows.iter().all(|r| {
                r.as_array().is_some_and(|p| p.len() == 2 && is_hash16(&p[0]) && p[1].as_str().is_some_and(|l| l.chars().count() <= LOG_STRING_MAX))
            }),
            (_, Value::Null | Value::Bool(_) | Value::Number(_)) => true,
            (_, Value::String(text)) => text.chars().count() <= LOG_STRING_MAX,
            _ => false,
        };
        if !ok {
            return None;
        }
        clean.insert(key.clone(), value.clone());
    }
    // Worker-local ids restart in a new worker: prefixed with the session and cell they belong to.
    let scope = format!("{}/{}", stamp.get("session").and_then(Value::as_str).unwrap_or(""), stamp.get("cell").map(Value::to_string).unwrap_or_default());
    for key in ["call_id", "chunk"] {
        if let Some(Value::String(id)) = clean.get(key) {
            let scoped = format!("{scope}/{id}");
            clean.insert(key.into(), json!(scoped));
        }
    }
    for key in STAMP_KEYS {
        clean.remove(key);
    }
    clean.extend(stamp.iter().map(|(k, v)| (k.clone(), v.clone())));
    serde_json::to_string(&Value::Object(clean)).ok()
}

/// `FACTR_CLASSIFY_LOG`: `{pid}` expands to this engine's process id; a directory gets `classify-<pid>.jsonl`.
fn log_path(setting: &str, pid: u32) -> PathBuf {
    let path = PathBuf::from(setting.replace("{pid}", &pid.to_string()));
    if path.is_dir() { path.join(format!("classify-{pid}.jsonl")) } else { path }
}

static LOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static LOG_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn append_rows(path: &Path, rows: &[String]) -> Result<()> {
    use std::io::Write;
    let _one_writer = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("{}: refusing to write the classify log through a symlink", path.display());
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        bail!("{}: the classify log must be a regular file", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o600 {
            bail!("{}: the classify log exists with mode {mode:o}, not 600; refusing to append to a file others can read or write", path.display());
        }
    }
    for row in rows {
        // One write per row: with O_APPEND a row is never interleaved with another engine's.
        let mut bytes = row.clone().into_bytes();
        bytes.push(b'\n');
        file.write_all(&bytes)?;
    }
    Ok(())
}

/// Append the worker's `classify` log rows (hashes and counts only; anything else is dropped) to
/// `FACTR_CLASSIFY_LOG`. A write failure is reported once on stderr and never fails the cell.
fn classify_log(lines: &str, stamp: &serde_json::Map<String, Value>) -> Result<()> {
    let Some(setting) = std::env::var("FACTR_CLASSIFY_LOG").ok().filter(|p| !p.trim().is_empty()) else {
        return Ok(());
    };
    let rows: Vec<String> = lines.lines().filter_map(|line| clean_log_row(line, stamp)).collect();
    if rows.is_empty() {
        return Ok(());
    }
    if let Err(err) = append_rows(&log_path(&setting, std::process::id()), &rows) {
        if !LOG_FAILED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("factr: the classify log could not be written: {err:#}");
        }
    }
    Ok(())
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
/// Runs up to [`batch_concurrency`] sub-queries at once and returns a JSON array of replies in
/// order; a failed item becomes an error string, not a failure of the call.
pub async fn llm_query_batch(llm_query: &LlmQuery, prompts_json: &str) -> Result<String> {
    llm_query_batch_with(llm_query, prompts_json, batch_concurrency()).await
}

async fn llm_query_batch_with(llm_query: &LlmQuery, prompts_json: &str, concurrency: usize) -> Result<String> {
    let q = llm_query.clone();
    let results = run_batch(prompts_json, concurrency, move |p| q(p)).await?;
    let replies: Vec<String> = results
        .into_iter()
        .map(|(_, _, r)| match r {
            Ok(text) => text,
            Err(err) => format!("Error: {err:#}"),
        })
        .collect();
    Ok(serde_json::to_string(&replies)?)
}

/// Like [`llm_query_batch`], but each reply is an object `{"t": text, "e": error or null, "i": input
/// tokens, "o": output tokens, "c": cached tokens, "r": reasoning tokens, "ms": latency, "s": ms after the
/// batch started that the call got its concurrency slot, "f": effort the call ran at, "q": effort it asked
/// for, "x": ms a first request took that the API refused for its effort, or null}` (a token count the
/// provider did not report is null; a failed call's latency is the time it took to fail). Used by `classify`.
pub async fn llm_query_batch_meta(meta: &LlmQueryMeta, prompts_json: &str, concurrency: usize) -> Result<String> {
    let m = meta.clone();
    let results = run_batch(prompts_json, concurrency, move |p| m(p)).await?;
    let replies: Vec<Value> = results
        .into_iter()
        .map(|(at, took, r)| match r {
            Ok(r) => json!({"t": r.text, "e": null, "i": r.input_tokens, "o": r.output_tokens, "c": r.cached_tokens, "r": r.reasoning_tokens, "ms": r.latency_ms, "s": at, "f": r.effort, "q": r.requested_effort, "x": r.refused_ms}),
            Err(err) => json!({"t": "", "e": format!("Error: {err:#}"), "i": null, "o": null, "c": null, "r": null, "ms": took, "s": at, "f": null, "q": null, "x": null}),
        })
        .collect();
    Ok(serde_json::to_string(&replies)?)
}

/// A [`LlmQueryMeta`] over a plain [`LlmQuery`]: timed, with no usage.
pub fn meta_from_plain(plain: LlmQuery) -> LlmQueryMeta {
    Arc::new(move |prompt: String| {
        let plain = plain.clone();
        Box::pin(async move {
            let started = Instant::now();
            let text = plain(prompt).await?;
            Ok(SubReply { text, latency_ms: started.elapsed().as_millis() as u64, ..Default::default() })
        })
    })
}

/// Every prompt through `call`, at most `concurrency` at once, in order: (ms after the batch started that the
/// call got its slot, ms it took, result).
async fn run_batch<T: Send + 'static>(
    prompts_json: &str,
    concurrency: usize,
    call: impl Fn(String) -> BoxFuture<Result<T>>,
) -> Result<Vec<(u64, u64, Result<T>)>> {
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
    let gate = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut jobs = tokio::task::JoinSet::new();
    let batch_start = Instant::now();
    for (index, prompt) in prompts.into_iter().enumerate() {
        let started = within_query_cap(prompt).map(|p| call(p));
        let gate = gate.clone();
        jobs.spawn(async move {
            let _slot = gate.acquire_owned().await;
            let at = batch_start.elapsed();
            let result = match started {
                Ok(call) => call.await,
                Err(err) => Err(err),
            };
            let took = batch_start.elapsed().saturating_sub(at);
            (index, at.as_millis() as u64, took.as_millis() as u64, result)
        });
    }
    let mut replies: Vec<Option<(u64, u64, Result<T>)>> = (0..jobs.len()).map(|_| None).collect();
    while let Some(done) = jobs.join_next().await {
        let (index, at, took, result) = done.context("llm_query_batch task failed")?;
        replies[index] = Some((at, took, result));
    }
    Ok(replies.into_iter().map(|r| r.expect("every job reported")).collect())
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
        assert_eq!(batch_allowance("not json", 8), Duration::ZERO);
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 8]), 8), BATCH_WAVE_ALLOWANCE);
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 64]), 8), BATCH_WAVE_ALLOWANCE * 8);
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

    fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| vars.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string())
    }

    #[test]
    fn cost_switches_default_to_the_new_behaviour_and_parse_strictly() {
        let cfg = classify_cfg(&env_of(&[]), "low", "m");
        assert_eq!(cfg, json!({"legacy": false, "format": "json", "chunk_items": null, "chunk_chars": null, "dedupe": false, "log": false, "concurrency": 8, "effort": "low", "model": "m"}));
        let set = [
            ("FACTR_CLASSIFY_FORMAT", "CODES"), ("FACTR_CLASSIFY_CHUNK_ITEMS", "80"), ("FACTR_CLASSIFY_CHUNK_CHARS", "30000"),
            ("FACTR_CLASSIFY_DEDUPE", "1"), ("FACTR_CLASSIFY_LOG", "/tmp/x.jsonl"), ("FACTR_BATCH_CONCURRENCY", "16"),
        ];
        let cfg = classify_cfg(&env_of(&set), "", "");
        assert_eq!(cfg, json!({"legacy": false, "format": "codes", "chunk_items": 80, "chunk_chars": 30000, "dedupe": true, "log": true, "concurrency": 16, "effort": "", "model": ""}));
        // Out of range or garbage falls back to the default, never to something silly.
        let bad = [("FACTR_CLASSIFY_CHUNK_ITEMS", "0"), ("FACTR_CLASSIFY_CHUNK_CHARS", "9999999"), ("FACTR_BATCH_CONCURRENCY", "x"), ("FACTR_CLASSIFY_FORMAT", "yaml")];
        let cfg = classify_cfg(&env_of(&bad), "", "");
        assert_eq!((cfg["chunk_items"].as_u64(), cfg["chunk_chars"].as_u64(), cfg["concurrency"].as_u64(), cfg["format"].as_str()), (None, None, Some(8), Some("json")));
        assert_eq!(concurrency_from(&env_of(&[("FACTR_BATCH_CONCURRENCY", "65")])), 8);
    }

    #[test]
    fn the_master_switch_restores_the_0_0_3_values() {
        let all = [("FACTR_COST_LEGACY", "1"), ("FACTR_BATCH_CONCURRENCY", "16"), ("FACTR_REPL_SUB_EFFORT", "none")];
        assert_eq!(concurrency_from(&env_of(&all)), 8);
        assert_eq!(classify_cfg(&env_of(&all), "", "")["legacy"], true);
        assert!(!legacy_from(&env_of(&[("FACTR_COST_LEGACY", "0")])));
        assert_eq!(resolve_sub_effort(None, Some("none"), true), Ok(None));
    }

    #[test]
    fn sub_effort_defaults_to_inherit_and_rejects_unknown_values() {
        assert_eq!(resolve_sub_effort(None, None, false), Ok(None), "never hard-coded: the main agent's effort");
        assert_eq!(resolve_sub_effort(Some("low"), None, false), Ok(Some("low".into())), "a cheaper effort is an explicit setting");
        assert_eq!(resolve_sub_effort(Some("high"), None, false), Ok(Some("high".into())));
        assert_eq!(resolve_sub_effort(Some("high"), Some(" None "), false), Ok(Some("none".into())), "the env beats the config");
        assert_eq!(resolve_sub_effort(Some(""), Some(""), false), Ok(None));
        assert_eq!(resolve_sub_effort(Some("inherit"), None, false), Ok(None));
        for bad in ["minimal", "max", "swarm", "fast", "lo"] {
            let err = resolve_sub_effort(None, Some(bad), false).unwrap_err();
            assert!(err.contains(bad) && err.contains("none|low|medium|high|xhigh") && err.contains("main agent's effort"), "{err}");
        }
    }

    #[tokio::test]
    async fn batch_concurrency_is_a_parameter_and_the_wait_follows_it() {
        let (live, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let prompts: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
        let out = llm_query_batch_with(&counting(live, peak.clone()), &json_list(&prompts), 3).await.unwrap();
        assert_eq!(serde_json::from_str::<Vec<String>>(&out).unwrap().len(), 20);
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 64]), 16), BATCH_WAVE_ALLOWANCE * 8, "16-wide never earns less than 8-wide");
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 64]), 4), BATCH_WAVE_ALLOWANCE * 16, "narrower earns more");
        assert_eq!(batch_allowance(&json_list(&vec!["x".to_string(); 64]), 0), BATCH_WAVE_ALLOWANCE * 64, "never divides by zero");
    }

    #[tokio::test]
    async fn the_meta_batch_returns_usage_per_reply_in_order_and_errors_as_objects() {
        let meta: LlmQueryMeta = Arc::new(|prompt: String| {
            Box::pin(async move {
                if prompt == "bad" {
                    return Err(anyhow!("429"));
                }
                Ok(SubReply { text: prompt.to_uppercase(), input_tokens: Some(10), output_tokens: Some(2), cached_tokens: Some(0), reasoning_tokens: None, latency_ms: 7, effort: Some("low".into()), ..Default::default() })
            })
        });
        let out = llm_query_batch_meta(&meta, &json_list(&["a".into(), "bad".into(), "c".into()]), 2).await.unwrap();
        let rows: Vec<Value> = serde_json::from_str(&out).unwrap();
        assert!(rows[0]["s"].is_u64(), "the slot time is reported: {}", rows[0]);
        assert_eq!(rows[0], json!({"t": "A", "e": null, "i": 10, "o": 2, "c": 0, "r": null, "ms": 7, "s": rows[0]["s"], "f": "low", "q": null, "x": null}));
        assert_eq!(rows[1]["e"], "Error: 429");
        assert!(rows[1]["ms"].is_u64() && rows[1]["s"].is_u64(), "a failed call still has its timing: {}", rows[1]);
        assert_eq!(rows[2]["t"], "C");
        // Without a metered closure the plain query is timed and carries no usage.
        let plain = meta_from_plain(counting(Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))));
        let out = llm_query_batch_meta(&plain, &json_list(&["p1".into()]), 2).await.unwrap();
        let rows: Vec<Value> = serde_json::from_str(&out).unwrap();
        assert_eq!((rows[0]["t"].as_str(), rows[0]["i"].is_null(), rows[0]["e"].is_null()), (Some("p1:2"), true, true));
    }

    #[test]
    fn log_rows_are_validated_and_stripped_host_side() {
        let call = r#"{"type":"call","call_id":"1.1","classify_call":1,"pass":0,"wave":1,"attempt":1,"chunk_size":2,"records_count":2,"input_tokens":5,"output_tokens":null,"effort":"low","format":"codes","error":"reject: missing","refusal":false,"transport_error":false,"validation_failure":false,"results":[["0123456789abcdef","spam"]],"prompt":"SECRET TEXT"}"#;
        let stamp = log_stamp("sess", 4, Some("item-7"));
        let clean: Value = serde_json::from_str(&clean_log_row(call, &stamp).unwrap()).unwrap();
        assert!(clean.get("prompt").is_none(), "unknown keys are dropped");
        assert_eq!((clean["results"][0][1].as_str(), clean["error"].as_str(), clean["call_id"].as_str()), (Some("spam"), Some("reject: missing"), Some("sess/4/1.1")));
        let occ = r#"{"type":"occ","classify_call":1,"occ":3,"h":"0123456789abcdef","label":"spam","truncated":false,"cached":false,"deduped":true,"chunk":"1.1","pos":0,"text":"x","session":"forged"}"#;
        let clean: Value = serde_json::from_str(&clean_log_row(occ, &stamp).unwrap()).unwrap();
        assert_eq!(clean, json!({"type":"occ","classify_call":1,"occ":3,"h":"0123456789abcdef","label":"spam","truncated":false,"cached":false,"deduped":true,"chunk":"sess/4/1.1","pos":0,"session":"sess","cell":4,"run_id":"item-7"}));
        let job = r#"{"type":"job","classify_call":1,"status":"failed","unlabelled":3,"error":"transport: 503"}"#;
        let clean: Value = serde_json::from_str(&clean_log_row(job, &log_stamp("s", 1, None)).unwrap()).unwrap();
        assert_eq!((clean["status"].as_str(), clean["unlabelled"].as_i64(), clean.get("run_id").is_none()), (Some("failed"), Some(3), true));
        for bad in [
            "not json",
            r#"{"type":"other"}"#,
            r#"{"event":"call","call_id":"1.1"}"#,
            r#"{"type":"occ","occ":0,"h":"short","label":"x"}"#,
            r#"{"type":"occ","occ":0,"h":"0123456789abcdef","label":{"a":1}}"#,
            r#"{"type":"call","results":[["0123456789abcdef","a","extra"]]}"#,
            r#"{"type":"call","results":[["not-a-hash","a"]]}"#,
        ] {
            assert!(clean_log_row(bad, &stamp).is_none(), "{bad}");
        }
        let long = format!(r#"{{"type":"occ","occ":0,"h":"0123456789abcdef","label":"{}"}}"#, "x".repeat(LOG_STRING_MAX + 1));
        assert!(clean_log_row(&long, &stamp).is_none(), "a long string is not a label");
        assert!(clean_log_row(&format!(r#"{{"type":"call","model":"{}"}}"#, "y".repeat(LOG_ROW_MAX)), &stamp).is_none());
        // The engine counts cells per session; a worker restart does not reset them.
        assert_eq!((next_cell("cells-a"), next_cell("cells-a"), next_cell("cells-b")), (1, 2, 1));
    }

    #[test]
    fn a_backoff_sleep_never_outlasts_the_cells_remaining_wait() {
        assert_eq!(sleep_allowed(1_500, Duration::from_secs(120)), 1_500);
        assert_eq!(sleep_allowed(60_000, Duration::from_secs(120)), MAX_SLEEP_MS, "capped per request");
        assert_eq!(sleep_allowed(5_000, Duration::from_millis(700)), 700, "never past the cell's wait");
        assert_eq!(sleep_allowed(5_000, Duration::ZERO), 0, "an exhausted wait cannot be slept on");
    }

    #[test]
    fn the_log_path_expands_pid_and_a_directory_gets_a_per_engine_file() {
        assert_eq!(log_path("/tmp/x-{pid}.jsonl", 7), PathBuf::from("/tmp/x-7.jsonl"));
        let dir = std::env::temp_dir().join(format!("logdir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(log_path(dir.to_str().unwrap(), 9), dir.join("classify-9.jsonl"));
        let file = dir.join("one.jsonl");
        assert_eq!(log_path(file.to_str().unwrap(), 9), file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_log_is_private_append_only_one_row_per_write_and_never_through_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("logw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.jsonl");
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for n in 0..50 {
                        append_rows(&path, &[format!(r#"{{"type":"call","call_id":"{}","attempt":{n}}}"#, t * 100 + n)]).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 200);
        assert!(text.lines().all(|l| serde_json::from_str::<Value>(l).is_ok()), "no interleaved rows");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // A log someone else can read or write is refused, not appended to.
        let open = dir.join("open.jsonl");
        std::fs::write(&open, "").unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = append_rows(&open, &["{}".into()]).unwrap_err().to_string();
        assert!(err.contains("mode 644"), "{err}");
        assert_eq!(std::fs::read_to_string(&open).unwrap(), "");
        let link = dir.join("link.jsonl");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let err = append_rows(&link, &["{}".into()]).unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 200);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
