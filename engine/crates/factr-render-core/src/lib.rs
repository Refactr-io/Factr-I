//! # factr-render-core
//!
//! Reasoning-line markdown helpers shared by the server/streaming path and the
//! session renderer (`factr-base`).

pub mod reasoning;

pub use reasoning::{
    REASONING_SENTINEL, reasoning_line_content, reasoning_line_markup, reasoning_partial_markup,
    reasoning_summary_line_markup,
};
