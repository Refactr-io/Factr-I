#![allow(
    unknown_lints,
    clippy::collapsible_match,
    clippy::manual_checked_ops,
    clippy::unnecessary_sort_by,
    clippy::useless_conversion
)]
// Tests hold the process-wide env lock across awaits on purpose.
#![cfg_attr(test, allow(clippy::await_holding_lock))]

//! Shared application core and the Factr desktop's headless runtime.

pub use factr_app_core::*;
pub mod cli_auth;
pub mod factr_runtime;
pub mod shutdown;
