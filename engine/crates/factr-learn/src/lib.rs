//! Recursive REPL and continual-learning harness for the factr engine.
//! Rust owns provider and host calls; CPython runs in a sandboxed per-session
//! worker using the Python runtime bundled with Factr.

pub mod agent_loop;
pub mod agent_loop_host;
mod bundled_skills;
pub use bundled_skills::{install as install_shipped_skills, shipped_dirs as shipped_skill_dirs};
pub mod goal_ratchet;
pub mod entries;
pub mod host;
pub use factr_base::migrate;
pub mod refine;
pub mod skill_files;
mod worker;

pub use host::{LlmQuery, ReplHost, RunOutput};

/// Tool description shown to the model; kept short because it is sent on every request.
pub const TOOL_DESCRIPTION: &str =
    "Python REPL: load, llm_query_batch; sandboxed (macOS) or approved per cell.";
