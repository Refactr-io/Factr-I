//! Messages injected into one call's request (stop nudges, the turn's system reminder, repeat-guard
//! and deadline reminders, unsaved memory). They are never written to the session. When `sticky`
//! is on they stay at the position they were first sent at for the rest of the turn, so the input
//! of the next call extends the input of this one and a provider can continue its chain instead
//! of resending the whole history uncached. Off, each one is sent once at the tail, as before.

use crate::message::Message;

#[derive(Default)]
pub(super) struct StickyInjections {
    sticky: bool,
    /// (position in the sent request, message), in the order they were first sent.
    kept: Vec<(usize, Message)>,
    /// Provider history length and the hash of its last message at the previous call. A history
    /// that no longer starts with that (compaction, a rewrite) drops every kept position.
    base: Option<(usize, u64)>,
    reminder_kept: bool,
}

impl StickyInjections {
    pub(super) fn new(sticky: bool) -> Self {
        Self { sticky, ..Self::default() }
    }

    /// The history to send this call: `history` with every kept message back at its position.
    pub(super) fn start_call(&mut self, history: &[Message], compacted: bool) -> Vec<Message> {
        if !self.sticky {
            return history.to_vec();
        }
        let still_extends = !compacted
            && self.base.is_none_or(|(len, hash)| {
                len <= history.len() && (len == 0 || last_hash(&history[..len]) == hash)
            });
        if !still_extends {
            // ponytail: drops the overlay on any rewrite (one full request), rebase if that is common.
            self.kept.clear();
            self.reminder_kept = false;
        }
        self.base = Some((history.len(), last_hash(history)));
        let mut out = history.to_vec();
        for (pos, message) in &self.kept {
            out.insert((*pos).min(out.len()), message.clone());
        }
        out
    }

    /// Append an injected message to this call's request (and keep it there when sticky).
    pub(super) fn push(&mut self, out: &mut Vec<Message>, message: Message) {
        if self.sticky {
            self.kept.push((out.len(), message.clone()));
        }
        out.push(message);
    }

    /// The turn's system reminder: sent on every call; when sticky, sent once and then kept.
    pub(super) fn push_reminder(&mut self, out: &mut Vec<Message>, reminder: Option<Message>) {
        let Some(reminder) = reminder else { return };
        if self.reminder_kept {
            return;
        }
        self.reminder_kept = self.sticky;
        self.push(out, reminder);
    }
}

fn last_hash(history: &[Message]) -> u64 {
    history.last().map_or(0, factr_provider_core::fingerprint::stable_hash_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .map(|m| serde_json::to_string(&m.content).unwrap())
            .collect()
    }

    #[test]
    fn kept_messages_stay_in_place_and_the_next_call_extends_the_last() {
        let mut history = vec![Message::user("task"), Message::assistant_text("DONE")];
        let mut sticky = StickyInjections::new(true);
        let mut first = sticky.start_call(&history, false);
        sticky.push_reminder(&mut first, Some(Message::user("<system-reminder>r</system-reminder>")));
        sticky.push(&mut first, Message::user("<system-reminder>nudge</system-reminder>"));
        history.push(Message::assistant_text("reading"));
        let mut second = sticky.start_call(&history, false);
        sticky.push_reminder(&mut second, Some(Message::user("<system-reminder>r</system-reminder>")));
        assert_eq!(texts(&second[..first.len()]), texts(&first));
        assert_eq!(second.len(), first.len() + 1, "the reminder is not sent twice");

        // A compaction (or a rewritten history) drops the kept positions and resends the reminder.
        let mut third = sticky.start_call(&history[..1], false);
        sticky.push_reminder(&mut third, Some(Message::user("<system-reminder>r</system-reminder>")));
        assert_eq!(third.len(), 2);
        let mut fourth = sticky.start_call(&history[..1], true);
        assert_eq!(fourth.len(), 1);
        sticky.push_reminder(&mut fourth, Some(Message::user("<system-reminder>r</system-reminder>")));
        assert_eq!(fourth.len(), 2);
    }

    #[test]
    fn off_sends_each_message_once_at_the_tail() {
        let mut history = vec![Message::user("task")];
        let mut plain = StickyInjections::new(false);
        let mut first = plain.start_call(&history, false);
        plain.push_reminder(&mut first, Some(Message::user("r")));
        plain.push(&mut first, Message::user("nudge"));
        history.push(Message::assistant_text("reading"));
        let mut second = plain.start_call(&history, false);
        plain.push_reminder(&mut second, Some(Message::user("r")));
        assert_eq!(texts(&second), texts(&[history[0].clone(), history[1].clone(), Message::user("r")]));
    }
}
