//! Switches for the shape of OpenAI Responses requests. `BASELINE` is the request the engine
//! has always sent; `VARIANT` matches the reference clients (the Codex CLI and the Python agent).

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestShape {
    pub strict_tools: bool,
    pub chatgpt_cache_key: bool,
    pub parallel_tool_calls: bool,
    pub replay_reasoning_id: bool,
    /// Experiment (`FACTR_OPENAI_CACHE_KEY_MODE=content`): in ChatGPT mode the key is a hash of
    /// the instructions and the sorted tools, with no session id, so sessions that share a
    /// prompt and tool set share a key. `session` (false) keeps the per-session key.
    pub content_cache_key: bool,
}

impl RequestShape {
    pub const BASELINE: Self = Self {
        strict_tools: true,
        chatgpt_cache_key: false,
        parallel_tool_calls: false,
        replay_reasoning_id: true,
        content_cache_key: false,
    };
    pub const VARIANT: Self = Self {
        strict_tools: false,
        chatgpt_cache_key: true,
        parallel_tool_calls: false,
        replay_reasoning_id: true,
        content_cache_key: false,
    };

    /// `FACTR_VARIANT_BASELINE=1` wins over everything; otherwise each switch set to `0` or `1`
    /// overrides its `VARIANT` default and any other value is ignored.
    pub fn resolve(env: &dyn Fn(&str) -> Option<String>) -> Self {
        if env("FACTR_VARIANT_BASELINE").as_deref() == Some("1") {
            return Self::BASELINE;
        }
        let mut shape = Self::VARIANT;
        for (name, field) in [
            ("FACTR_OPENAI_STRICT_TOOLS", &mut shape.strict_tools),
            (
                "FACTR_OPENAI_CHATGPT_CACHE_KEY",
                &mut shape.chatgpt_cache_key,
            ),
            (
                "FACTR_OPENAI_PARALLEL_TOOLS",
                &mut shape.parallel_tool_calls,
            ),
            (
                "FACTR_OPENAI_REPLAY_REASONING_ID",
                &mut shape.replay_reasoning_id,
            ),
        ] {
            match env(name).as_deref() {
                Some("0") => *field = false,
                Some("1") => *field = true,
                _ => {}
            }
        }
        match env("FACTR_OPENAI_CACHE_KEY_MODE").as_deref() {
            Some("session") => shape.content_cache_key = false,
            Some("content") => shape.content_cache_key = true,
            _ => {}
        }
        shape
    }

    /// ChatGPT mode: the idle prewarm leaves a per-session key out of the warmup and of the
    /// adoption check (it cannot know the session); a content key is sent and compared.
    pub fn prewarm_omits_cache_key(&self, is_chatgpt_mode: bool) -> bool {
        is_chatgpt_mode && self.chatgpt_cache_key && !self.content_cache_key
    }

    /// Read once per process, so the prewarm payload, the first request, retries and every
    /// continuation use the same shape.
    pub fn current() -> &'static Self {
        static SHAPE: OnceLock<RequestShape> = OnceLock::new();
        SHAPE.get_or_init(|| Self::resolve(&|name| std::env::var(name).ok()))
    }
}

/// Switches for the agent loop around the requests (what is injected, what is echoed back).
/// `BASELINE` is the loop the engine has always run; `FACTR_VARIANT_BASELINE=1` selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopShape {
    /// Messages injected into one call's request (nudges, reminders, unsaved memory) stay at
    /// the same position in every later call of the turn, so the next input extends this one.
    pub sticky_nudges: bool,
    /// A user instruction not to run the result stops the auto-verify gate and the deadline's
    /// "verify" wording; one not to check it also stops the verify nudge.
    pub respect_no_verify: bool,
    /// When every unexecuted write came from the edit tools, the verify nudge does not ask for a
    /// read-back (the content is already in context).
    pub nudge_no_reread: bool,
    /// The model-facing copy of a successful write/edit/apply_patch/replace result leaves out
    /// the appended full diff; the session and the UI keep it.
    pub compact_write_result: bool,
    /// Tool descriptions steer reads, searches and edits to the file tools instead of the shell.
    pub tool_steering: bool,
    /// Deadline in seconds for one REPL sub-model call (`llm_query`, each `llm_query_batch` item,
    /// each `classify` chunk); `0` is none. A call past it fails as a timeout, which `classify`
    /// asks again once, instead of holding the cell until its whole wait runs out.
    pub repl_subcall_timeout_s: u64,
    /// A REPL batch (`llm_query_batch`, a `classify` wave) that runs out of the cell's model-call wait
    /// returns the replies that came back and fails only the missing items, instead of failing whole.
    pub repl_keep_partial_wave: bool,
}

impl LoopShape {
    pub const BASELINE: Self = Self {
        sticky_nudges: false,
        respect_no_verify: false,
        nudge_no_reread: false,
        compact_write_result: false,
        tool_steering: false,
        repl_subcall_timeout_s: 0,
        repl_keep_partial_wave: false,
    };
    pub const VARIANT: Self = Self {
        sticky_nudges: true,
        respect_no_verify: true,
        nudge_no_reread: true,
        compact_write_result: true,
        tool_steering: true,
        repl_subcall_timeout_s: 90,
        repl_keep_partial_wave: true,
    };

    /// Same rules as [`RequestShape::resolve`].
    pub fn resolve(env: &dyn Fn(&str) -> Option<String>) -> Self {
        if env("FACTR_VARIANT_BASELINE").as_deref() == Some("1") {
            return Self::BASELINE;
        }
        let mut shape = Self::VARIANT;
        for (name, field) in [
            ("FACTR_LOOP_STICKY_NUDGES", &mut shape.sticky_nudges),
            ("FACTR_LOOP_RESPECT_NO_VERIFY", &mut shape.respect_no_verify),
            ("FACTR_LOOP_NUDGE_NO_REREAD", &mut shape.nudge_no_reread),
            (
                "FACTR_LOOP_COMPACT_WRITE_RESULT",
                &mut shape.compact_write_result,
            ),
            ("FACTR_LOOP_TOOL_STEERING", &mut shape.tool_steering),
            ("FACTR_REPL_KEEP_PARTIAL_WAVE", &mut shape.repl_keep_partial_wave),
        ] {
            match env(name).as_deref() {
                Some("0") => *field = false,
                Some("1") => *field = true,
                _ => {}
            }
        }
        if let Some(secs) = env("FACTR_REPL_SUBCALL_TIMEOUT_S").and_then(|v| v.parse().ok()) {
            shape.repl_subcall_timeout_s = secs;
        }
        shape
    }

    /// Read once per process.
    pub fn current() -> &'static Self {
        static SHAPE: OnceLock<LoopShape> = OnceLock::new();
        SHAPE.get_or_init(|| Self::resolve(&|name| std::env::var(name).ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolve(vars: &[(&str, &str)]) -> RequestShape {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        RequestShape::resolve(&|name| vars.get(name).cloned())
    }

    #[test]
    fn baseline_master_wins_over_every_switch() {
        assert_eq!(resolve(&[]), RequestShape::VARIANT);
        assert_eq!(
            resolve(&[
                ("FACTR_VARIANT_BASELINE", "1"),
                ("FACTR_OPENAI_STRICT_TOOLS", "0"),
                ("FACTR_OPENAI_CHATGPT_CACHE_KEY", "1"),
                ("FACTR_OPENAI_PARALLEL_TOOLS", "1"),
                ("FACTR_OPENAI_REPLAY_REASONING_ID", "0"),
            ]),
            RequestShape::BASELINE
        );
        // Only the exact value 1 turns the master switch on.
        assert_eq!(
            resolve(&[("FACTR_VARIANT_BASELINE", "true")]),
            RequestShape::VARIANT
        );
    }

    #[test]
    fn each_switch_flips_exactly_one_field() {
        let v = RequestShape::VARIANT;
        let cases = [
            (
                "FACTR_OPENAI_STRICT_TOOLS",
                RequestShape {
                    strict_tools: !v.strict_tools,
                    ..v
                },
            ),
            (
                "FACTR_OPENAI_CHATGPT_CACHE_KEY",
                RequestShape {
                    chatgpt_cache_key: !v.chatgpt_cache_key,
                    ..v
                },
            ),
            (
                "FACTR_OPENAI_PARALLEL_TOOLS",
                RequestShape {
                    parallel_tool_calls: !v.parallel_tool_calls,
                    ..v
                },
            ),
            (
                "FACTR_OPENAI_REPLAY_REASONING_ID",
                RequestShape {
                    replay_reasoning_id: !v.replay_reasoning_id,
                    ..v
                },
            ),
        ];
        // The cache-key mode takes words, not 0/1.
        let content = RequestShape {
            content_cache_key: true,
            ..v
        };
        assert_eq!(resolve(&[("FACTR_OPENAI_CACHE_KEY_MODE", "content")]), content);
        assert_eq!(resolve(&[("FACTR_OPENAI_CACHE_KEY_MODE", "session")]), v);
        assert_eq!(
            resolve(&[
                ("FACTR_VARIANT_BASELINE", "1"),
                ("FACTR_OPENAI_CACHE_KEY_MODE", "content")
            ]),
            RequestShape::BASELINE
        );
        for junk in ["", "1", "Content", "global"] {
            assert_eq!(resolve(&[("FACTR_OPENAI_CACHE_KEY_MODE", junk)]), v, "{junk:?}");
        }
        for (name, flipped) in cases {
            let (off, on) = (resolve(&[(name, "0")]), resolve(&[(name, "1")]));
            // One value keeps the default, the other differs in this field only.
            assert!(
                (off, on) == (v, flipped) || (off, on) == (flipped, v),
                "{name}"
            );
        }
    }

    fn resolve_loop(vars: &[(&str, &str)]) -> LoopShape {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        LoopShape::resolve(&|name| vars.get(name).cloned())
    }

    const LOOP_SWITCHES: [&str; 6] = [
        "FACTR_LOOP_STICKY_NUDGES",
        "FACTR_LOOP_RESPECT_NO_VERIFY",
        "FACTR_LOOP_NUDGE_NO_REREAD",
        "FACTR_LOOP_COMPACT_WRITE_RESULT",
        "FACTR_LOOP_TOOL_STEERING",
        "FACTR_REPL_KEEP_PARTIAL_WAVE",
    ];

    /// VARIANT with only the field behind `name` flipped.
    fn loop_flipped(name: &str) -> LoopShape {
        let mut s = LoopShape::VARIANT;
        let field = match name {
            "FACTR_LOOP_STICKY_NUDGES" => &mut s.sticky_nudges,
            "FACTR_LOOP_RESPECT_NO_VERIFY" => &mut s.respect_no_verify,
            "FACTR_LOOP_NUDGE_NO_REREAD" => &mut s.nudge_no_reread,
            "FACTR_LOOP_COMPACT_WRITE_RESULT" => &mut s.compact_write_result,
            "FACTR_LOOP_TOOL_STEERING" => &mut s.tool_steering,
            "FACTR_REPL_KEEP_PARTIAL_WAVE" => &mut s.repl_keep_partial_wave,
            _ => unreachable!("{name}"),
        };
        *field = !*field;
        s
    }

    #[test]
    fn loop_baseline_master_wins_and_each_loop_switch_flips_one_field() {
        assert_eq!(resolve_loop(&[]), LoopShape::VARIANT);
        let all_on: Vec<_> = LOOP_SWITCHES.iter().map(|n| (*n, "1")).collect();
        let mut master = all_on.clone();
        master.push(("FACTR_VARIANT_BASELINE", "1"));
        assert_eq!(resolve_loop(&master), LoopShape::BASELINE);
        for name in LOOP_SWITCHES {
            let (off, on) = (resolve_loop(&[(name, "0")]), resolve_loop(&[(name, "1")]));
            let (v, flipped) = (LoopShape::VARIANT, loop_flipped(name));
            assert!((off, on) == (v, flipped) || (off, on) == (flipped, v), "{name}");
            for junk in ["", "true", "2"] {
                assert_eq!(resolve_loop(&[(name, junk)]), LoopShape::VARIANT, "{name}={junk:?}");
            }
        }
    }

    #[test]
    fn the_sub_call_deadline_takes_seconds_and_baseline_has_none() {
        assert_eq!(resolve_loop(&[]).repl_subcall_timeout_s, 90);
        assert_eq!(resolve_loop(&[("FACTR_REPL_SUBCALL_TIMEOUT_S", "0")]).repl_subcall_timeout_s, 0);
        assert_eq!(resolve_loop(&[("FACTR_REPL_SUBCALL_TIMEOUT_S", "45")]).repl_subcall_timeout_s, 45);
        for junk in ["", "-1", "1.5", " 45"] {
            assert_eq!(resolve_loop(&[("FACTR_REPL_SUBCALL_TIMEOUT_S", junk)]), LoopShape::VARIANT, "{junk:?}");
        }
        let master = [("FACTR_VARIANT_BASELINE", "1"), ("FACTR_REPL_SUBCALL_TIMEOUT_S", "45")];
        assert_eq!(resolve_loop(&master), LoopShape::BASELINE);
    }

    #[test]
    fn junk_values_are_ignored() {
        for name in [
            "FACTR_OPENAI_STRICT_TOOLS",
            "FACTR_OPENAI_CHATGPT_CACHE_KEY",
            "FACTR_OPENAI_PARALLEL_TOOLS",
            "FACTR_OPENAI_REPLAY_REASONING_ID",
        ] {
            for junk in ["", "true", "yes", "2", " 1"] {
                assert_eq!(
                    resolve(&[(name, junk)]),
                    RequestShape::VARIANT,
                    "{name}={junk:?}"
                );
            }
        }
    }
}
