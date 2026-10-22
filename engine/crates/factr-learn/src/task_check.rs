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

fn trigger_commands(task: &str) -> Vec<String> {    let lower = task.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    for trigger in TRIGGERS {
        let mut from = 0;
        while let Some(at) = lower[from..].find(trigger) {
            let (begin, start) = (from + at, from + at + trigger.len());
            from = start;
            // "Never test with ./prod.sh" forbids; it does not name a check.
            let before = &lower[begin.saturating_sub(30)..begin];
            let before = before.rsplit(['.', '\n']).next().unwrap_or(before);
            if NEGATIONS.iter().any(|n| before.contains(n)) {
                continue;
            }
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


/// Words that make a sentence about verifying the work.
const VERBS: [&str; 7] = ["test", "check", "verify", "validate", "evaluate", "run", "try"];
/// A sentence that forbids the action ("do not run `./deploy.sh`") names no check.
const NEGATIONS: [&str; 6] = ["do not", "don't", "never", "avoid", "without", "must not"];
const CONNECTORS: [&str; 5] = ["with", "using", "via", "through", "running"];
/// Programs whose arguments make a command line a plausible check (`pytest -x`, `cargo test`, `make check`).
const RUNNERS: [&str; 10] = ["pytest", "cargo", "go", "npm", "pnpm", "yarn", "make", "tox", "nox", "ctest"];

/// A command line that can be a check: a local program or script (`./eval`, `scripts/t.sh`, `check.py`), an
/// interpreter with a file, or a known runner with arguments.
fn command_like(cmd: &str) -> bool {
    let mut words = cmd.split_whitespace();
    let first = words.next().unwrap_or("");
    if first.is_empty() || cmd.len() > MAX_LEN {
        return false;
    }
    if looks_like_command(first) && !INTERPRETERS.contains(&first) {
        return true;
    }
    if INTERPRETERS.contains(&first) {
        return words.next().is_some_and(|f| !f.starts_with('-'));
    }
    RUNNERS.contains(&first) && words.next().is_some()
}

/// The sentences of `text`: split at `.`, `!` and `?` followed by whitespace, and at newlines, never inside
/// backticks (so `./eval` and `check.py` stay whole).
fn sentences(text: &str) -> Vec<String> {
    let (mut out, mut cur, mut ticks) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' {
            ticks = !ticks;
        }
        let ends = !ticks && (c == '\n' || (matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace())));
        if ends {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Commands a sentence names as the way to verify the work: a backticked command line that can be a check, or
/// a path-like token after `with`/`using`/`by running`, in a sentence that has a verification verb and no negation.
fn sentence_checks(sentence: &str) -> Vec<String> {
    let lower = sentence.to_ascii_lowercase();
    if NEGATIONS.iter().any(|n| lower.contains(n)) {
        return Vec::new();
    }
    let verbal = |text: &str| text.split(|c: char| !c.is_ascii_alphabetic()).any(|w| VERBS.contains(&w));
    // The verb must be in the prose, not inside a backticked name (`tests/test_a.py`).
    let prose: String = lower.split('`').step_by(2).collect::<Vec<_>>().join(" ");
    if !verbal(&prose) {
        return Vec::new();
    }
    let mut found = Vec::new();
    // Backticked command lines.
    let parts: Vec<&str> = sentence.split('`').collect();
    for (i, part) in parts.iter().enumerate().skip(1).step_by(2) {
        let cmd = part.trim();
        if i + 1 < parts.len() && command_like(cmd) {
            found.push(cmd.to_string());
        }
    }
    if !found.is_empty() {
        return found;
    }
    // An unquoted path-like token right after a connector, complete only when the sentence ends or moves on.
    let words: Vec<&str> = sentence.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if !CONNECTORS.contains(&w.to_ascii_lowercase().as_str()) {
            continue;
        }
        let Some(first_raw) = words.get(i + 1) else { continue };
        let first = clean(first_raw);
        if !looks_like_command(first) || INTERPRETERS.contains(&first) {
            continue;
        }
        if closes(words.get(i + 2).map(|w| clean(w)), first_raw) {
            found.push(first.to_string());
        }
    }
    found
}

/// Commands the task text names for checking the work: at most three, each complete and at most 200 chars.
/// Two forms: an explicit trigger phrase ("test with ./eval"), or a sentence about verifying that names a
/// command line in backticks or after `with`/`using` ("Test your file with the provided command `./eval`").
pub fn task_check_commands(task: &str) -> Vec<String> {
    let mut out = trigger_commands(task);
    for sentence in sentences(task) {
        for cmd in sentence_checks(&sentence) {
            if (3..=MAX_LEN).contains(&cmd.len()) && !out.contains(&cmd) && out.len() < MAX_CHECKS {
                out.push(cmd);
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

    const TRITON_STYLE: &str = "Deliverable: write ONE file named `solution.py` in the current working directory. It must define the kernel(s).\n\nThis machine has no GPU. Test your file with the provided command `./eval` (run it from the current directory). It runs solution.py on a remote GPU and prints JSON: call_acc (the file runs without error), exec_acc (outputs match the reference). You may run it as often as you like. Stop when exec_acc is true or you cannot improve further. Do not use the web or any other machine.";

    #[test]
    fn a_sentence_about_verifying_names_its_command_in_many_styles() {
        assert_eq!(task_check_commands(TRITON_STYLE), vec!["./eval"]);
        for (text, want) in [
            ("Verify your solution with `make check`.", vec!["make check"]),
            ("Check it by running ./check.sh when you are done.", vec!["./check.sh"]),
            ("You can validate the result using `python3 validate.py --strict`.", vec!["python3 validate.py --strict"]),
            ("Run `pytest -x tests/` to check your changes.", vec!["pytest -x tests/"]),
            ("When finished, evaluate your answer with the supplied script `scripts/grade.sh`.", vec!["scripts/grade.sh"]),
            ("Please test using ./run_tests.sh and fix what fails.", vec!["./run_tests.sh"]),
            ("Make the tests pass.\nTry `cargo test -p core` after each change.", vec!["cargo test -p core"]),
        ] {
            assert_eq!(task_check_commands(text), want, "{text}");
        }
    }

    #[test]
    fn sentences_that_forbid_describe_or_only_mention_files_name_no_check() {
        for text in [
            "Do not run `./deploy.sh` under any circumstances.",
            "Never test with ./prod.sh.",
            "Run `rm -rf build` before you start.",
            "Use `git status` to check what changed.",
            "Check `README.md` for details.",
            "The tests live in `tests/test_a.py`.",
            "Write the file `solution.py` and keep it short.",
            "You can run it as often as you like with `ls`.",
        ] {
            assert!(task_check_commands(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn triggers_and_sentences_do_not_duplicate_and_stay_bounded() {
        assert_eq!(task_check_commands("Test with ./eval. Then verify using `./eval`."), vec!["./eval"]);
        let many = "Check with `./a.sh`. Check with `./b.sh`. Check with `./c.sh`. Check with `./d.sh`.";
        assert_eq!(task_check_commands(many).len(), 3);
    }
}
