//! Factr's own Python backend as an on-demand feature service.
//!
//! The Rust harness owns the hot path (chat, tools, memory, sessions). The
//! ~200 other Factr features (cron, profiles, vault, messaging,
//! voice, ...) are served by Factr's real Python backend, started only when
//! one of them is first used and stopped again after an idle period, so a
//! chat-only session never loads Python. The backend binds loopback with its
//! own random token; that token never leaves this process.

use anyhow::{Context, Result, bail};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify};

/// Factr can take a while on a cold start (it measured ~1.6 s warm here).
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// The Python backend costs hundreds of MB resident and starts in about 1.7 s (measured cold: 1.55-1.79 s), so it
/// stops after 1 idle minute. An enabled messaging platform, or a cron job that runs more often than this, keeps it up.
pub const IDLE_STOP_AFTER: Duration = Duration::from_secs(60);

pub fn idle_stop_after() -> Duration {
    std::env::var("FACTR_FEATURE_IDLE_MS")
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(IDLE_STOP_AFTER)
}

const LOG_CAP: u64 = 2 * 1024 * 1024;
const TAIL_LINES: usize = 20;

/// Copy the backend's stderr to `path` (rotated to `path.1` past [`LOG_CAP`], keeping one old file),
/// returning the last few lines for the "exited before ready" error. Finishes when the child closes stderr.
fn capture_stderr(stderr: tokio::process::ChildStderr, path: Option<std::path::PathBuf>) -> tokio::task::JoinHandle<Vec<String>> {
    use std::io::Write;
    tokio::spawn(async move {
        let open = |p: &std::path::Path| std::fs::OpenOptions::new().create(true).append(true).open(p).ok();
        let mut file = path.as_deref().and_then(|p| {
            std::fs::create_dir_all(p.parent()?).ok()?;
            open(p)
        });
        let mut size = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
        let mut tail = std::collections::VecDeque::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let (Some(f), Some(p)) = (file.as_mut(), path.as_deref()) {
                if size >= LOG_CAP {
                    let _ = std::fs::rename(p, p.with_extension("log.1"));
                    if let Some(fresh) = open(p) {
                        *f = fresh;
                        size = 0;
                    }
                }
                if writeln!(f, "{line}").is_ok() {
                    size += line.len() as u64 + 1;
                }
            }
            if tail.len() == TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        tail.into()
    })
}

struct Running {
    child: Child,
    port: u16,
}

/// Drop guard from [`Features::lease`].
pub struct Lease(std::sync::Arc<Features>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.leases.fetch_sub(1, Ordering::Relaxed);
        self.0.touch("lease-released", "lease");
    }
}

pub struct Features {
    /// Program + leading args (e.g. `["/…/factr"]`); `serve …` is appended.
    command: Vec<String>,
    pub token: String,
    running: Mutex<Option<Running>>,
    /// Milliseconds since `epoch` of the last forwarded call.
    last_used_ms: AtomicU64,
    epoch: Instant,
    /// Work in flight that must not be idle-stopped (a cron tick running jobs); see [`Lease`].
    leases: AtomicUsize,
    /// Rung when a forwarded call touched the cron API, so the cron timer re-reads `jobs.json`.
    pub cron_changed: Notify,
    /// This gateway's own REST base URL and auth token, so cron (`run_job`)
    /// can call `/api/agent/run` instead of its own `AIAgent`. `None` until
    /// `set_engine_env` runs, right after the gateway binds its port.
    engine_env: std::sync::Mutex<Option<(String, String)>>,
    /// Consecutive failed bot-backend starts and when the last one failed (see [`Features::ensure_bots`]).
    bot_failures: std::sync::Mutex<(u32, Option<Instant>)>,
    /// The one bundled-skills seed attempt of this process (see [`Features::seed_bundled_skills_once`]).
    skills_seed: tokio::sync::OnceCell<bool>,
}

/// A one-shot `skills seed` copies ~60 small skill folders; it measured ~0.5 s.
const SKILLS_SEED_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a new chat on an empty skill store waits for that seed, so its first turn sees the
/// bundled skills; a slower seed finishes in the background and the chat starts anyway.
pub const SKILLS_SEED_CHAT_WAIT: Duration = Duration::from_secs(3);

/// Whether the one skill store (`$FACTR_HOME/skills`) holds no skill at all.
fn skill_store_is_empty() -> bool {
    factr_base::skill::SkillRegistry::load_global().map_or(true, |registry| registry.list().is_empty())
}

/// `copied` from the last JSON line `skills seed` prints (`{"copied": n, ...}`); 0 when absent.
fn parse_seed_copied(stdout: &str) -> u64 {
    stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok()?["copied"].as_u64())
        .unwrap_or(0)
}

/// Wait after `failures` failed bot-backend starts: 1 minute, doubling, capped at 30.
fn bot_retry_after(failures: u32) -> Duration {
    Duration::from_secs(60 << failures.saturating_sub(1).min(5)).min(Duration::from_secs(30 * 60))
}

impl Features {
    pub fn new(command: Vec<String>) -> Self {
        Self {
            command,
            token: crate::auth::generate_token(),
            running: Mutex::new(None),
            last_used_ms: AtomicU64::new(0),
            epoch: Instant::now(),
            leases: AtomicUsize::new(0),
            cron_changed: Notify::new(),
            engine_env: std::sync::Mutex::new(None),
            bot_failures: std::sync::Mutex::new((0, None)),
            skills_seed: tokio::sync::OnceCell::new(),
        }
    }

    /// Seed Factr's bundled skills into `$FACTR_HOME/skills` when the store is empty, at most once
    /// per process: a fresh desktop install otherwise lists no skills until the Python backend
    /// happens to start for some other feature. Lazy by design: called from the skills list and a
    /// new desktop chat, never at engine start, and only where a Factr backend is configured (a
    /// benchmark run has none, so it never gets here). Runs `<backend> skills seed`, a one-shot
    /// that exits; the long-lived feature backend is not started. Returns whether skills were
    /// copied; callers re-read the registry then.
    pub async fn seed_bundled_skills_once(&self) -> bool {
        *self
            .skills_seed
            .get_or_init(|| async {
                if !skill_store_is_empty() {
                    return false;
                }
                match self.run_skills_seed().await {
                    Ok(copied) => copied > 0,
                    Err(err) => {
                        eprintln!("factr: seeding the bundled skills failed: {err:#}");
                        false
                    }
                }
            })
            .await
    }

    /// For a new chat: when the skill store is empty, run the one-shot seed and wait for it at most
    /// `wait` (it keeps running past that). Returns whether skills were copied within the wait.
    /// A store that already has skills returns at once and spawns nothing.
    pub async fn seed_bundled_skills_for_chat(self: &std::sync::Arc<Self>, wait: Duration) -> bool {
        if !skill_store_is_empty() {
            return false;
        }
        let features = std::sync::Arc::clone(self);
        let seed = tokio::spawn(async move { features.seed_bundled_skills_once().await });
        matches!(tokio::time::timeout(wait, seed).await, Ok(Ok(true)))
    }

    async fn run_skills_seed(&self) -> Result<u64> {
        let (program, args) = self.command.split_first().context("no Factr backend command configured")?;
        let mut command = Command::new(program);
        #[cfg(unix)]
        command.process_group(0);
        if let Ok(pythonpath) = std::env::var("FACTR_BACKEND_PYTHONPATH") {
            command.env("PYTHONPATH", pythonpath);
        }
        if let Ok(dir) = factr_base::storage::factr_dir() {
            command.env("FACTR_HOME", dir);
        }
        let child = command
            .args(args)
            .args(["skills", "seed"])
            .env_remove("FACTR_DASHBOARD_SESSION_TOKEN")
            .env_remove("FACTR_ENGINE_TOKEN")
            .env("FACTR_DESKTOP", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting `{program} skills seed`"))?;
        let output = tokio::time::timeout(SKILLS_SEED_TIMEOUT, child.wait_with_output())
            .await
            .context("`skills seed` did not finish in time")??;
        if !output.status.success() {
            bail!("`skills seed` exited with {}", output.status);
        }
        Ok(parse_seed_copied(&String::from_utf8_lossy(&output.stdout)))
    }

    /// Record the engine's own REST URL and token, passed to the Python
    /// backend on its next (re)spawn as `FACTR_ENGINE_URL`/`_TOKEN`.
    pub fn set_engine_env(&self, url: String, token: String) {
        *self.engine_env.lock().unwrap_or_else(|e| e.into_inner()) = Some((url, token));
    }

    /// Note use of the backend now (a frame crossed an open socket).
    pub fn mark_active(&self) {
        self.last_used_ms.store(self.epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    pub fn touch(&self, route: &str, source: &str) {
        self.mark_active();
        if route.starts_with("/api/cron") || route.starts_with("cron.") {
            self.cron_changed.notify_one();
        }
        if std::env::var_os("FACTR_TRACE_FEATURE_ACTIVITY").is_some() {
            eprintln!("feature_activity at={:?} route={} source={}", SystemTime::now(), route, source);
        }
    }

    /// Keep the backend alive until the returned guard drops.
    pub fn lease(self: &std::sync::Arc<Self>) -> Lease {
        self.leases.fetch_add(1, Ordering::Relaxed);
        Lease(self.clone())
    }

    pub(crate) fn idle_for(&self) -> Duration {
        let last = Duration::from_millis(self.last_used_ms.load(Ordering::Relaxed));
        self.epoch.elapsed().saturating_sub(last)
    }

    /// Port of the running backend, starting it if needed.
    pub async fn port(&self) -> Result<u16> {
        self.touch("backend-start-or-reuse", "feature-port");
        let mut running = self.running.lock().await;
        if let Some(r) = running.as_mut() {
            if r.child.try_wait().ok().flatten().is_none() {
                return Ok(r.port);
            }
            *running = None; // exited: start a fresh one
        }
        let (program, args) = self.command.split_first().context("no Factr backend command configured")?;
        let mut command = Command::new(program);
        #[cfg(unix)]
        command.process_group(0);
        if let Ok(pythonpath) = std::env::var("FACTR_BACKEND_PYTHONPATH") {
            command.env("PYTHONPATH", pythonpath);
        }
        if let Ok(tools_dir) = std::env::var("FACTR_BACKEND_TOOLS_DIR") {
            let mut paths = vec![std::path::PathBuf::from(tools_dir)];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            command.env("PATH", std::env::join_paths(paths)?);
        }
        // Secrets go on stdin (`--secrets-stdin`), never the environment: `ps eww` prints a child's env.
        let mut secrets = serde_json::json!({ "session_token": self.token });
        if let Some((url, token)) = self.engine_env.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            command.env("FACTR_ENGINE_URL", url);
            secrets["engine_token"] = token.into();
        }
        // Where the engine's skill registry reads: without it Python's skill writes land in FACTR_CONFIG_HOME.
        if let Ok(dir) = factr_base::storage::factr_dir() {
            command.env("FACTR_HOME", dir);
        }
        let mut child = command.args(args)
            .args(["serve", "--host", "127.0.0.1", "--port", "0", "--skip-build", "--secrets-stdin"])
            .env_remove("FACTR_DASHBOARD_SESSION_TOKEN")
            .env_remove("FACTR_ENGINE_TOKEN")
            // Desktop-owned backend. With FACTR_ENGINE_URL set (below) it runs no cron
            // ticker: the engine's timer fires `POST /api/cron/tick` at the due time.
            .env("FACTR_DESKTOP", "1")
            // Factr's parent-death watchdog: exit within ~2 s if the engine
            // dies, even by SIGKILL (kill_on_drop only covers clean exits).
            // A start marker without a matching nonce would disarm it, so drop
            // any inherited from the desktop that launched us.
            .env("FACTR_PARENT_PID", std::process::id().to_string())
            .env_remove("FACTR_PARENT_START_MARKER")
            .env_remove("FACTR_PARENT_NONCE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting the Factr feature backend ({program})"))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A child that already died is reported by the readiness wait below.
            let _ = stdin.write_all(format!("{secrets}\n").as_bytes()).await;
        }
        let log = factr_base::storage::factr_dir().ok().map(|d| d.join("logs").join("factr-backend.log"));
        let stderr = capture_stderr(child.stderr.take().context("backend stderr")?, log);
        let mut lines = BufReader::new(child.stdout.take().context("backend stdout")?).lines();
        let port = tokio::time::timeout(START_TIMEOUT, async {
            while let Some(line) = lines.next_line().await? {
                if let Some(rest) = line.split("FACTR_BACKEND_READY port=").nth(1) {
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    return Ok::<u16, anyhow::Error>(digits.parse()?);
                }
            }
            let tail = tokio::time::timeout(Duration::from_secs(2), stderr).await.ok().and_then(Result::ok).unwrap_or_default();
            bail!("the Factr feature backend exited before it was ready:\n{}", tail.join("\n"))
        })
        .await
        .context("the Factr feature backend did not become ready in time")??;
        // Keep draining stdout so a chatty backend never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        *running = Some(Running { child, port });
        Ok(port)
    }

    pub async fn is_running(&self) -> bool {
        let mut running = self.running.lock().await;
        match running.as_mut() {
            Some(r) => r.child.try_wait().ok().flatten().is_none(),
            None => false,
        }
    }

    /// Stop the backend if it has been idle for `IDLE_STOP_AFTER`.
    pub async fn stop_if_idle(&self) {
        if self.leases.load(Ordering::Relaxed) > 0 {
            return;
        }
        if let Some(home) = factr_base::factr_config::home() {
            if messaging_enabled(&home) {
                return; // inbound bot traffic never touches the proxy; stopping now would deafen the bots
            }
        }
        if self.idle_for() < idle_stop_after() {
            if std::env::var_os("FACTR_TRACE_FEATURE_ACTIVITY").is_some() { eprintln!("feature_idle decision=keep idle_ms={} threshold_ms={}", self.idle_for().as_millis(), idle_stop_after().as_millis()); }
            return;
        }
        if std::env::var_os("FACTR_TRACE_FEATURE_ACTIVITY").is_some() { eprintln!("feature_idle decision=stop idle_ms={}", self.idle_for().as_millis()); }
        if let Some(mut r) = self.running.lock().await.take() {
            kill_backend(&mut r.child).await;
        }
    }

    /// Bots only receive messages while the backend runs and nothing else starts it: when the
    /// config in `home` has a platform enabled, start it if it isn't running (boot, or it died).
    pub async fn ensure_bots(&self, home: &std::path::Path) {
        if messaging_enabled(home) && !self.is_running().await {
            let (failures, last) = *self.bot_failures.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|at| at.elapsed() < bot_retry_after(failures)) {
                return; // a backend that cannot start is not retried every tick
            }
            let started = self.port().await;
            let mut state = self.bot_failures.lock().unwrap_or_else(|e| e.into_inner());
            match started {
                Ok(_) => *state = (0, None),
                Err(err) => {
                    *state = (failures + 1, Some(Instant::now()));
                    eprintln!("factr: could not start the messaging backend (next try in {}s): {err:#}", bot_retry_after(failures + 1).as_secs());
                }
            }
        }
    }

    pub async fn stop(&self) {
        if let Some(mut r) = self.running.lock().await.take() {
            kill_backend(&mut r.child).await;
        }
    }
}

/// True when the config the desktop's messaging page writes (`platforms.<id>.enabled`
/// in `config.yaml`, for the home or any profile) has a platform switched on.
pub fn messaging_enabled(home: &std::path::Path) -> bool {
    let mut files = vec![home.join("config.yaml")];
    if let Ok(entries) = std::fs::read_dir(home.join("profiles")) {
        files.extend(entries.flatten().map(|e| e.path().join("config.yaml")));
    }
    files.iter().any(|f| {
        std::fs::read_to_string(f)
            .ok()
            .and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(&raw).ok())
            .and_then(|cfg| cfg["platforms"].as_mapping().cloned())
            .is_some_and(|m| m.values().any(|p| p["enabled"].as_bool() == Some(true)))
    })
}

async fn kill_backend(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The Python backend can leave worker subprocesses behind; own a process
        // group so idle-stop reclaims the entire feature service tree.
        // SIGTERM first so it can finish a write; SIGKILL the group if it lingers.
        let _ = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        if tokio::time::timeout(Duration::from_secs(3), child.wait()).await.is_err() {
            let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            let _ = child.wait().await;
        }
        return;
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in backend: announces a port, then stays alive.
    #[cfg(unix)]
    fn fake_backend(dir: &std::path::Path, announce: &str) -> Vec<String> {
        let script = dir.join("fake-factr");
        std::fs::write(&script, format!("#!/bin/sh\necho 'starting'\necho '{announce}'\nexec sleep 30\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        vec![script.to_string_lossy().into_owned()]
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn backend_stderr_is_logged_rotated_and_tailed() {
        let dir = std::env::temp_dir().join(format!("features-stderr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = dir.join("logs/factr-backend.log");
        let mut child = Command::new("sh")
            .args(["-c", "echo 'ModuleNotFoundError: no yaml' >&2; head -c 3000000 /dev/zero | tr '\\0' x | fold -w 100 >&2; echo >&2; echo last >&2"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let tail = capture_stderr(child.stderr.take().unwrap(), Some(log.clone())).await.unwrap();
        assert_eq!(tail.last().map(String::as_str), Some("last"));
        assert!(std::fs::metadata(&log).unwrap().len() <= LOG_CAP + 200, "the live log stays under the cap");
        assert!(dir.join("logs/factr-backend.log.1").exists(), "the old log is kept once");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The backend reads its tokens from stdin; neither is in its environment or argv.
    #[cfg(unix)]
    #[tokio::test]
    async fn backend_tokens_arrive_on_stdin_not_env() {
        let dir = std::env::temp_dir().join(format!("features-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("seen");
        let script = dir.join("fake-factr");
        std::fs::write(&script, format!(
            "#!/bin/sh\nread line\n{{ echo \"$line\"; env; echo \"$@\"; }} > '{}'\necho 'FACTR_BACKEND_READY port=4244'\nexec sleep 30\n",
            out.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let f = Features::new(vec![script.to_string_lossy().into_owned()]);
        f.set_engine_env("http://127.0.0.1:1".into(), "engine-secret".into());
        assert_eq!(f.port().await.unwrap(), 4244);
        let seen = std::fs::read_to_string(&out).unwrap();
        let (stdin_line, rest) = seen.split_once('\n').unwrap();
        assert!(stdin_line.contains(&f.token) && stdin_line.contains("engine-secret"));
        assert!(!rest.contains(&f.token) && !rest.contains("engine-secret"), "no token in env or argv");
        assert!(rest.contains("--secrets-stdin") && rest.contains("FACTR_ENGINE_URL=http://127.0.0.1:1"));
        f.stop_if_idle().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starts_on_demand_reuses_and_stops_when_idle() {
        let dir = std::env::temp_dir().join(format!("features-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = Features::new(fake_backend(&dir, "FACTR_BACKEND_READY port=4242"));
        assert!(!f.is_running().await, "nothing runs until a feature is used");
        assert_eq!(f.port().await.unwrap(), 4242);
        assert!(f.is_running().await);
        assert_eq!(f.port().await.unwrap(), 4242, "reused, not respawned");
        f.stop_if_idle().await;
        assert!(f.is_running().await, "recently used: kept");
        f.last_used_ms.store(0, Ordering::Relaxed);
        let f2 = Features { epoch: Instant::now() - IDLE_STOP_AFTER - Duration::from_secs(1), ..f };
        f2.stop_if_idle().await;
        assert!(!f2.is_running().await, "idle: stopped");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bots_start_the_backend_and_a_leased_one_is_never_idle_stopped() {
        let dir = std::env::temp_dir().join(format!("features-bots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = std::sync::Arc::new(Features::new(fake_backend(&dir, "FACTR_BACKEND_READY port=4243")));
        f.ensure_bots(&dir).await;
        assert!(!f.is_running().await, "no bot enabled: stays off");
        std::fs::write(dir.join("config.yaml"), "platforms:\n  telegram:\n    enabled: true\n").unwrap();
        f.ensure_bots(&dir).await;
        assert!(f.is_running().await, "bot enabled: started without any feature call");
        // A request in flight holds a lease: idle time alone must not stop the backend.
        let f = std::sync::Arc::new(Features { epoch: Instant::now() - IDLE_STOP_AFTER - Duration::from_secs(9), ..std::sync::Arc::try_unwrap(f).ok().unwrap() });
        std::fs::remove_file(dir.join("config.yaml")).unwrap();
        let lease = f.lease();
        f.stop_if_idle().await;
        assert!(f.is_running().await, "leased: kept");
        drop(lease);
        f.last_used_ms.store(0, Ordering::Relaxed);
        f.stop_if_idle().await;
        assert!(!f.is_running().await, "lease released and idle: stopped");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_bot_backend_that_cannot_start_is_retried_with_growing_backoff_then_reset() {
        let dir = std::env::temp_dir().join(format!("features-backoff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), "platforms:\n  telegram:\n    enabled: true\n").unwrap();
        let (script, tries) = (dir.join("dies"), dir.join("tries"));
        std::fs::write(&script, format!("#!/bin/sh\necho x >> '{}'\nexit 1\n", tries.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let count = || std::fs::read_to_string(&tries).map_or(0, |t| t.lines().count());
        let f = Features::new(vec![script.to_string_lossy().into_owned()]);
        f.ensure_bots(&dir).await;
        f.ensure_bots(&dir).await;
        assert_eq!(count(), 1, "second tick is inside the 1 minute backoff");
        f.bot_failures.lock().unwrap().1 = Some(Instant::now() - Duration::from_secs(61));
        f.ensure_bots(&dir).await;
        assert_eq!(count(), 2);
        f.bot_failures.lock().unwrap().1 = Some(Instant::now() - Duration::from_secs(61));
        f.ensure_bots(&dir).await;
        assert_eq!(count(), 2, "the second failure waits 2 minutes");
        assert_eq!([1, 2, 3, 6, 7, 40].map(|n| bot_retry_after(n).as_secs()), [60, 120, 240, 1800, 1800, 1800]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn messaging_enabled_reads_platform_flags() {
        let dir = std::env::temp_dir().join(format!("features-msg-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("profiles/work")).unwrap();
        assert!(!messaging_enabled(&dir));
        std::fs::write(dir.join("config.yaml"), "platforms:\n  telegram:\n    enabled: false\n").unwrap();
        assert!(!messaging_enabled(&dir));
        std::fs::write(dir.join("profiles/work/config.yaml"), "platforms:\n  whatsapp:\n    enabled: true\n").unwrap();
        assert!(messaging_enabled(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn backend_gets_the_engine_factr_dir_and_a_sigterm_first() {
        // Other tests set FACTR_HOME under this lock; hold it so the expected dir can't move.
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let expected = factr_base::storage::factr_dir().unwrap();
        let dir = std::env::temp_dir().join(format!("features-term-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (env_out, term_out) = (dir.join("home"), dir.join("term"));
        let script = dir.join("fake-factr");
        std::fs::write(&script, format!(
            "#!/bin/sh\nprintf %s \"$FACTR_HOME\" > '{}'\ntrap 'echo t > \"{}\"; exit 0' TERM\necho FACTR_BACKEND_READY port=4244\nwhile :; do sleep 0.1; done\n",
            env_out.display(), term_out.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let f = Features::new(vec![script.to_string_lossy().into_owned()]);
        f.port().await.unwrap();
        f.stop().await;
        assert_eq!(std::fs::read_to_string(&env_out).unwrap(), expected.to_string_lossy());
        assert!(term_out.exists(), "stopped with SIGTERM, not straight SIGKILL");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_backend_that_never_announces_is_an_error() {
        let dir = std::env::temp_dir().join(format!("features-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("dies");
        std::fs::write(&script, "#!/bin/sh\necho nope\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let f = Features::new(vec![script.to_string_lossy().into_owned()]);
        assert!(f.port().await.unwrap_err().to_string().contains("exited before"));
        assert!(Features::new(vec![]).port().await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }


    #[test]
    fn seed_output_reports_the_copied_count() {
        assert_eq!(parse_seed_copied("noise\n{\"copied\": 58, \"total_bundled\": 58}\n"), 58);
        assert_eq!(parse_seed_copied("not json"), 0);
    }

    /// An empty skill store is seeded once through `<backend> skills seed`; a store that already
    /// has skills is left alone and nothing is spawned.
    #[cfg(unix)]
    #[tokio::test]
    async fn bundled_skills_are_seeded_once_into_an_empty_store() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("features-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("engine");
        std::fs::create_dir_all(&home).unwrap();
        let previous = std::env::var_os("FACTR_HOME");
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let (calls, script) = (dir.join("calls"), dir.join("fake-factr"));
        std::fs::write(&script, format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nmkdir -p \"$FACTR_HOME/skills/demo\"\nprintf -- '---\\nname: demo\\ndescription: d\\n---\\nbody\\n' > \"$FACTR_HOME/skills/demo/SKILL.md\"\necho '{{\"copied\": 1, \"total_bundled\": 1}}'\n",
            calls.display())).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let f = Features::new(vec![script.to_string_lossy().into_owned()]);
        assert!(f.seed_bundled_skills_once().await, "the empty store was seeded");
        assert!(f.seed_bundled_skills_once().await);
        assert_eq!(std::fs::read_to_string(&calls).unwrap(), "skills seed\n", "spawned once, as `skills seed`");
        assert!(!f.is_running().await, "the long-lived feature backend was not started");

        // A second process finds skills on disk: nothing to seed, nothing spawned.
        let g = Features::new(vec![script.to_string_lossy().into_owned()]);
        assert!(!g.seed_bundled_skills_once().await);
        assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1);

        match previous {
            Some(v) => unsafe { std::env::set_var("FACTR_HOME", v) },
            None => unsafe { std::env::remove_var("FACTR_HOME") },
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A new chat on an empty store waits (bounded) for the seed, so its first turn sees the skills;
    /// a seed slower than the wait does not hold the chat; a store with skills spawns nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_new_chat_waits_a_bounded_time_for_the_seed() {
        let _env = crate::factr_env::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("features-chat-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let previous = std::env::var_os("FACTR_HOME");
        let write_script = |name: &str, delay: &str| {
            let (calls, script) = (dir.join(format!("{name}.calls")), dir.join(name));
            std::fs::write(&script, format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\nsleep {delay}\nmkdir -p \"$FACTR_HOME/skills/demo\"\nprintf -- '---\\nname: demo\\ndescription: d\\n---\\nbody\\n' > \"$FACTR_HOME/skills/demo/SKILL.md\"\necho '{{\"copied\": 1, \"total_bundled\": 1}}'\n",
                calls.display())).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            (calls, script.to_string_lossy().into_owned())
        };

        // A quick seed: the chat waits for it and the store holds the skill afterwards.
        let home = dir.join("quick");
        std::fs::create_dir_all(&home).unwrap();
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let (calls, script) = write_script("quick-seed", "0.3");
        let f = std::sync::Arc::new(Features::new(vec![script.clone()]));
        assert!(f.seed_bundled_skills_for_chat(Duration::from_secs(3)).await, "seeded within the wait");
        assert!(home.join("skills/demo/SKILL.md").is_file(), "the first turn sees the seeded skill");
        // Skills on disk now: a second chat returns at once and spawns nothing.
        let g = std::sync::Arc::new(Features::new(vec![script]));
        let started = Instant::now();
        assert!(!g.seed_bundled_skills_for_chat(Duration::from_secs(3)).await);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1, "spawned once");

        // A slow seed: the chat stops waiting at the bound; the seed still finishes behind it.
        let home = dir.join("slow");
        std::fs::create_dir_all(&home).unwrap();
        unsafe { std::env::set_var("FACTR_HOME", &home) };
        let (_, script) = write_script("slow-seed", "2");
        let f = std::sync::Arc::new(Features::new(vec![script]));
        let started = Instant::now();
        assert!(!f.seed_bundled_skills_for_chat(Duration::from_millis(300)).await, "not within the wait");
        assert!(started.elapsed() < Duration::from_millis(1500), "the wait is bounded");
        assert!(f.seed_bundled_skills_once().await, "the seed finished in the background");
        assert!(home.join("skills/demo/SKILL.md").is_file());

        match previous {
            Some(v) => unsafe { std::env::set_var("FACTR_HOME", v) },
            None => unsafe { std::env::remove_var("FACTR_HOME") },
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
