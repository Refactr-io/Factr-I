//! The ordered, run-once cleanup both shutdown paths share (the signal thread and the runtime's own
//! signal future). Whichever gets there first does the work; the other waits for it, so the process
//! never exits halfway through draining spans or stopping commands.

use std::sync::Once;
use std::time::Duration;

static CLEANUP: Once = Once::new();

/// Let queued observability writes reach `factr.db`, then stop every command the engine started:
/// registered process groups first, then anything still carrying this engine's run token.
pub fn cleanup() {
    CLEANUP.call_once(|| {
        // Spans and run rows are written by a background thread a moment after a reply: wait for
        // the queue to drain (and stay drained briefly) so an immediate SIGTERM keeps them.
        factr_gateway::observability::flush_pending(Duration::from_millis(500), Duration::from_millis(2500));
        // The session venv this process made (best-effort, never inherited ones).
        crate::python_env::remove_session_venv();
        #[cfg(unix)]
        {
            // Reap the commands still running in-process: a bash that hit its timeout was promoted to
            // a background task and would otherwise outlive the engine.
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                rt.block_on(factr_app_core::background::global().abort_live_tasks_for_reload());
            }
            factr_app_core::background::kill_registered_process_groups(Duration::from_millis(500));
            // Then whatever escaped its group (setsid): found by the run token in its environment.
            factr_app_core::background::kill_run_token_descendants(Duration::from_millis(300));
            std::thread::sleep(Duration::from_millis(200)); // let the aborted tasks drop their children
        }
    });
}
