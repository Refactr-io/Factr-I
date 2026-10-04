//! `factr-base`: foundational layer of the factr application core.
//!
//! This crate holds the downward-closed set of modules that the upper
//! server/tool/agent layer (`factr-app-core`) depends on: provider, auth,
//! config, session, message, memory, and their supporting leaves.
//! Splitting it out lets the two halves compile as separate rustc units so the
//! largest compilation unit (and its peak memory) is roughly halved.
//!
//! `factr-app-core` re-exports this crate via `pub use factr_base::*`, so every
//! existing `crate::<module>` path in the upper layers keeps resolving.
// Tests hold the std env/home serialization lock across awaits on purpose.
#![cfg_attr(test, allow(clippy::await_holding_lock))]
#![allow(
    unknown_lints,
    clippy::collapsible_match,
    clippy::manual_checked_ops,
    clippy::unnecessary_sort_by,
    clippy::useless_conversion
)]

pub mod auth;
pub mod background;
pub mod bus;
pub mod cache_invalidation;
pub mod cache_tracker;
pub mod client_input;
pub mod compaction;
pub mod config;
pub mod console;
pub mod copilot_usage;
pub mod env;
pub mod real_home_guard;
pub mod factr_env;
pub mod external_auth;
pub mod github;
pub mod headless;
pub mod learn_signal;
pub mod factr_config;
pub mod factr_hooks;
pub mod hooks;
pub mod id;
pub mod import;
pub mod logging;
pub mod login_qr;
pub mod mcp;
pub mod memory;
pub mod memory_agent;
pub mod memory_expire;
pub mod memory_extract;
pub mod memory_graph;
pub mod memory_quality;
pub mod memory_recall;
pub mod migrate;
mod memory_store;
pub use memory_store::near_duplicate;
pub mod obs_sink;
pub use memory_store::migrate_factr_db;
pub mod memory_types;
pub mod message;
pub mod model_pricing;
pub mod model_usage;
pub mod output_style;
pub mod plan;
pub mod platform;
pub mod power_inhibit;
pub mod process_memory;
pub mod process_title;
pub mod python_env;
pub mod prompt;
pub mod protocol;
pub mod provider;
pub mod provider_activity;
pub mod provider_catalog;
pub mod recent_session_index;
pub mod registry;
pub mod runtime_memory_log;
pub mod secret_input;
pub mod session;
pub mod session_list_cache;
pub mod session_metrics;
pub mod skill;
pub mod soft_interrupt_store;
pub mod stdin_detect;
pub mod checkpoint_store;
pub mod storage;
pub mod terminal_launch;
pub mod todo;
pub mod transport;
pub mod usage;
pub mod util;
pub use factr_core::{terminal_eprint, terminal_eprintln, terminal_print, terminal_println};
