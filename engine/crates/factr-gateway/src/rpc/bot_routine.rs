//! Upgrade the old, safely quoted routine wrapper at the engine turn boundary. No shell is run.
use super::*;
use std::borrow::Cow;

fn quoted(input: &str) -> Result<(String, &str)> {
    let (mut output, mut rest) = (String::new(), input);
    anyhow::ensure!(rest.starts_with('\''), "invalid legacy routine argument");
    loop {
        rest = rest
            .strip_prefix('\'')
            .context("invalid legacy routine quote")?;
        let end = rest
            .find('\'')
            .context("unterminated legacy routine argument")?;
        output.push_str(&rest[..end]);
        rest = &rest[end + 1..];
        if let Some(next) = rest.strip_prefix("\"'\"") {
            output.push('\'');
            rest = next;
        } else {
            return Ok((output, rest));
        }
    }
}

pub(super) fn prompt<'a>(input: &'a str, profile: &str) -> Result<Cow<'a, str>> {
    let marker = "[bot-mode:routine:v2] ";
    let Some(start) = input.find(marker) else {
        return Ok(Cow::Borrowed(input));
    };
    let command = input[start..]
        .find("\n\nfactr -p ")
        .context("invalid legacy routine wrapper")?
        + start;
    let (owner, rest) = quoted(&input[command + "\n\nfactr -p ".len()..])?;
    anyhow::ensure!(
        owner == profile,
        "routine belongs to {owner}, but reached {profile}'s engine"
    );
    let (_, rest) = quoted(
        rest.strip_prefix(" chat -c ")
            .context("invalid legacy routine title")?,
    )?;
    let (instruction, rest) = quoted(
        rest.strip_prefix(" -q ")
            .context("invalid legacy routine instruction")?,
    )?;
    let suffix = "\n\nIf the command fails, report the error instead.";
    let tail = rest
        .strip_prefix(suffix)
        .context("invalid legacy routine suffix")?;
    Ok(Cow::Owned(format!(
        "{}{}{}",
        &input[..start],
        instruction,
        tail
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_quoted_routine_becomes_literal_instruction_on_its_owner_engine() {
        let input = "context\n[bot-mode:routine:v2] You are running the scheduled routine x\n\nfactr -p 'b' chat -c 'Routine: X' -q '[Scheduled routine] Literal $(x) `x` '\"'\"'word'\"'\"'\nnext'\n\nIf the command fails, report the error instead.\ntail";
        assert_eq!(
            prompt(input, "b").unwrap(),
            "context\n[Scheduled routine] Literal $(x) `x` 'word'\nnext\ntail"
        );
        assert!(prompt(input, "default").is_err());
        assert_eq!(prompt("ordinary prompt", "b").unwrap(), "ordinary prompt");
        assert!(prompt("[bot-mode:routine:v2] malformed", "b").is_err());
    }
}
