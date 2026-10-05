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

/// Tool description shown to the model once the tool is loaded (the tool is deferred, so this is
/// not in the first-request prefix). The limits quoted here are asserted against the constants in
/// `host.rs` by `tool_description_states_the_real_limits`; `OUTPUT_CLIP_CHARS` mirrors the clip
/// applied by the app-core tool.
pub const TOOL_DESCRIPTION: &str = "Persistent Python REPL: variables survive between calls, so keep large inputs and intermediate results in them. `await load(path, start=0, length=None)` reads a file slice (byte offsets). `await llm_query(prompt)` is one plain sub-model call (no tools); `await llm_query_batch(prompts)` runs up to 64 at once, 8 concurrently, 2000000 bytes in total. A prompt over 200000 chars is an error, not cut. The sub-model sees only the prompt text, so put the records in it. For judgment work over many records use `await classify(items, labels, guidance=None, votes=1)`: a validated label per item from the full label set, as a list aligned with items (chunked, one batch call per wave, bad replies re-asked, votes>1 re-asks disagreements); count it in code. Per cell: 16 host calls, 20 s compute, output clipped at 8000 chars. Always await helpers.";

/// Output clip the app-core REPL tool applies (`MAX_OUTPUT_CHARS` there), quoted in the description.
pub const OUTPUT_CLIP_CHARS: usize = 8_000;

#[cfg(test)]
mod description_tests {
    use super::*;

    #[test]
    fn tool_description_states_the_real_limits() {
        for n in [
            host::MAX_BATCH_PROMPTS.to_string(),
            host::BATCH_CONCURRENCY.to_string(),
            host::MAX_BATCH_BYTES.to_string(),
            host::MAX_QUERY_CHARS.to_string(),
            host::MAX_HOST_CALLS.to_string(),
            host::COMPUTE_TIMEOUT.as_secs().to_string(),
            OUTPUT_CLIP_CHARS.to_string(),
        ] {
            assert!(TOOL_DESCRIPTION.contains(&format!("{n} ")) || TOOL_DESCRIPTION.contains(&format!("{n}.")) || TOOL_DESCRIPTION.contains(&format!("{n},")), "{n} missing: {TOOL_DESCRIPTION}");
        }
        assert!(TOOL_DESCRIPTION.contains("load(path, start=0, length=None)"));
    }
}
