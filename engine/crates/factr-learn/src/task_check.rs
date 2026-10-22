//! The check a task names for itself ("test with ./eval", "check it with `make check`").
//!
//! Persistence needs a verification target the task owns; nothing is guessed from arbitrary output. Only a
//! command that follows an explicit trigger phrase is accepted, and only whole: a backtick-delimited command
//! is taken as written, an unquoted one only when the sentence clearly ends or moves on after it, so a
//! command with arguments the extractor cannot see is rejected rather than run shortened.

const MAX_CHECKS: usize = 3;
const MAX_LEN: usize = 200;
const TRIGGERS: [&str; 16] = [
    "test with", "tests with", "run the tests with", "run tests with", "check with", "check it with", "verify with",
    "verify it with", "validate with", "evaluate with", "evaluate using", "test using", "check using", "verify using",
    "run the evaluator", "evaluator is",
];
const INTERPRETERS: [&str; 8] = ["python", "python3", "node", "bash", "sh", "ruby", "perl", "make"];
/// A word that may follow a complete command in prose.
const AFTER_COMMAND: [&str; 16] = ["before", "when", "and", "then", "to", "until", "after", "once", "if", "so", "for", "on", "in", "at", "until", "while"];

fn clean(t: &str) -> &str {
    t.trim_start_matches(['"', '\'', '`', '(']).trim_end_matches(['.', ',', ';', ':', '"', '\'', '`', ')'])
}

fn looks_like_command(t: &str) -> bool {
    !t.is_empty()
        && (t.starts_with("./") || t.starts_with('/') || t.contains('/') || [".py", ".sh", ".js", ".rb", ".pl"].iter().any(|e| t.ends_with(e)) || INTERPRETERS.contains(&t))
}

/// Whether `word` ends the sentence or starts the next clause (so the command before it is complete).
fn closes(word: Option<&str>, last_raw: &str) -> bool {
    last_raw.ends_with(['.', ',', ';', ':', ')']) || word.is_none_or(|w| AFTER_COMMAND.contains(&w.to_ascii_lowercase().as_str()))
}

/// Commands the task text names for checking the work: at most three, each complete and at most 200 chars.
pub fn task_check_commands(task: &str) -> Vec<String> {
    let lower = task.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    for trigger in TRIGGERS {
        let mut from = 0;
        while let Some(at) = lower[from..].find(trigger) {
            let start = from + at + trigger.len();
            from = start;
            let rest = task[start..].trim_start();
            let cmd = if let Some(inner) = rest.strip_prefix('`') {
                inner.split('`').next().unwrap_or("").trim().to_string()
            } else {
                let mut words = rest.split_whitespace();
                let Some(first_raw) = words.next() else { continue };
                let first = clean(first_raw);
                if !looks_like_command(first) {
                    continue;
                }
                let (mut cmd, mut last_raw) = (first.to_string(), first_raw);
                if INTERPRETERS.contains(&first) {
                    let Some(file_raw) = words.next() else { continue };
                    let file = clean(file_raw);
                    if file.is_empty() || file.starts_with('-') {
                        continue;
                    }
                    cmd = format!("{first} {file}");
                    last_raw = file_raw;
                }
                if !closes(words.next().map(clean), last_raw) {
                    continue;
                }
                cmd
            };
            if (3..=MAX_LEN).contains(&cmd.len()) && !out.contains(&cmd) {
                out.push(cmd);
                if out.len() >= MAX_CHECKS {
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_check_is_taken_whole() {
        assert_eq!(task_check_commands("Write the code. Test with ./eval before you finish."), vec!["./eval"]);
        assert_eq!(task_check_commands("Check it with `make check --quiet -j2` please"), vec!["make check --quiet -j2"]);
        assert_eq!(task_check_commands("verify with python3 run_checks.py, then stop"), vec!["python3 run_checks.py"]);
        assert_eq!(task_check_commands("Run the tests with scripts/test.sh."), vec!["scripts/test.sh"]);
        assert_eq!(task_check_commands("Test with ./run_check.sh and make it pass."), vec!["./run_check.sh"]);
        assert_eq!(task_check_commands("Test with ./a.sh. Also check with `./b.sh`."), vec!["./a.sh", "./b.sh"]);
    }

    #[test]
    fn a_command_whose_arguments_cannot_be_seen_is_rejected_not_shortened() {
        assert!(task_check_commands("Test with ./eval --fast --device cuda before you finish").is_empty());
        assert!(task_check_commands("verify with python3 check.py --strict").is_empty());
    }

    #[test]
    fn prose_and_unrelated_text_name_no_check() {
        for none in ["test with care and check with your eyes", "verify with the user", "evaluate with a colleague", ""] {
            assert!(task_check_commands(none).is_empty(), "{none}");
        }
    }
}
