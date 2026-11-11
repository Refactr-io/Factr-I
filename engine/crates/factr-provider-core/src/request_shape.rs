//! Switches for the shape of OpenAI Responses requests. `BASELINE` is the request the engine
//! has always sent; `VARIANT` matches the reference clients (the Codex CLI and the Python agent).

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestShape {
    pub strict_tools: bool,
    pub chatgpt_cache_key: bool,
    pub parallel_tool_calls: bool,
    pub replay_reasoning_id: bool,
}

impl RequestShape {
    pub const BASELINE: Self = Self {
        strict_tools: true,
        chatgpt_cache_key: false,
        parallel_tool_calls: false,
        replay_reasoning_id: true,
    };
    pub const VARIANT: Self = Self {
        strict_tools: false,
        chatgpt_cache_key: true,
        parallel_tool_calls: false,
        replay_reasoning_id: true,
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
        shape
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
}

impl LoopShape {
    pub const BASELINE: Self = Self {
        sticky_nudges: false,
        respect_no_verify: false,
        nudge_no_reread: false,
        compact_write_result: false,
    };
    pub const VARIANT: Self = Self {
        sticky_nudges: true,
        respect_no_verify: true,
        nudge_no_reread: true,
        compact_write_result: true,
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
        ] {
            match env(name).as_deref() {
                Some("0") => *field = false,
                Some("1") => *field = true,
                _ => {}
            }
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

    const LOOP_SWITCHES: [&str; 4] = [
        "FACTR_LOOP_STICKY_NUDGES",
        "FACTR_LOOP_RESPECT_NO_VERIFY",
        "FACTR_LOOP_NUDGE_NO_REREAD",
        "FACTR_LOOP_COMPACT_WRITE_RESULT",
    ];

    /// VARIANT with only the field behind `name` flipped.
    fn loop_flipped(name: &str) -> LoopShape {
        let mut s = LoopShape::VARIANT;
        let field = match name {
            "FACTR_LOOP_STICKY_NUDGES" => &mut s.sticky_nudges,
            "FACTR_LOOP_RESPECT_NO_VERIFY" => &mut s.respect_no_verify,
            "FACTR_LOOP_NUDGE_NO_REREAD" => &mut s.nudge_no_reread,
            "FACTR_LOOP_COMPACT_WRITE_RESULT" => &mut s.compact_write_result,
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
