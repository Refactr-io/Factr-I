//! Rewind planning over the engine's history (undo, retry, branch count, prompt.submit truncation).
//!
//! factr's `rewind` addresses user and assistant messages only (tool rows are excluded), so every
//! position here is counted among those "rows". A row's durable id comes from `crate::undo::State`
//! (monotonic, never reused after an undo); `ids` below is one id per row. A session without an id
//! store falls back to [`position_ids`] (the 1-based position).

use serde_json::{Value, json};

#[derive(Debug)]
pub(crate) struct Refusal(pub i64, pub String);

fn err(code: i64, message: &str) -> Refusal {
    Refusal(code, message.into())
}

/// The user/assistant messages of an engine `get_history` reply.
pub(crate) fn rows(history: &Value) -> Vec<Value> {
    history["messages"]
        .as_array()
        .map(|all| all.iter().filter(|m| m["role"] == "user" || m["role"] == "assistant").cloned().collect())
        .unwrap_or_default()
}

/// Fallback ids for a session with no id store: the 1-based position.
pub(crate) fn position_ids(rows: &[Value]) -> Vec<u64> {
    (1..=rows.len() as u64).collect()
}

fn user_positions(rows: &[Value]) -> Vec<usize> {
    rows.iter().enumerate().filter(|(_, m)| m["role"] == "user").map(|(i, _)| i).collect()
}

/// Cutting the history at user turn `keep` (0-based; the turn itself is dropped).
pub(crate) struct Cut {
    /// factr `message_index` to rewind to; 0 means clear.
    pub index: usize,
    /// Rows dropped.
    pub removed: usize,
    /// Text of the first dropped user message.
    pub text: String,
}

pub(crate) fn cut_before_user(rows: &[Value], keep: usize) -> Option<Cut> {
    let at = *user_positions(rows).get(keep)?;
    Some(Cut { index: at, removed: rows.len() - at, text: rows[at]["content"].as_str().unwrap_or_default().to_string() })
}

/// The cut for `/undo N`: back up N user turns (at least one, at most all).
pub(crate) fn cut_last_users(rows: &[Value], n: usize) -> Option<Cut> {
    let users = user_positions(rows).len();
    cut_before_user(rows, users.checked_sub(n.clamp(1, users.max(1)))?)
}

/// The survivor payload of a cut at `index`: ids of the surviving user rows, and a map of every
/// surviving row id to itself (the client rebinds its cached ids from it).
pub(crate) fn survivors(rows: &[Value], ids: &[u64], index: usize) -> (Vec<u64>, Value) {
    let kept = &rows[..index.min(rows.len())];
    let users = user_positions(kept).into_iter().map(|i| ids[i]).collect();
    let map = kept.iter().enumerate().map(|(i, _)| (ids[i].to_string(), json!(ids[i]))).collect();
    (users, Value::Object(map))
}

/// What `prompt.submit` asked to truncate: `Ok(None)` for an ordinary submit, else the 0-based
/// user turn to cut before. Mirrors Factr's admission order and codes (4029 unconfirmed, 4018
/// stale target, 4030 ordinal drift, 4028 would empty the transcript).
pub(crate) fn truncation(p: &Value, rows: &[Value], ids: &[u64]) -> Result<Option<usize>, Refusal> {
    let ordinal = &p["truncate_before_user_ordinal"];
    let row = &p["truncate_before_row_id"];
    let message = &p["truncate_before_message_id"];
    if ordinal.is_null() && row.is_null() && message.is_null() {
        return Ok(None);
    }
    if p["confirm_truncate"].as_bool() != Some(true) {
        return Err(err(
            4029,
            "truncation parameters require confirm_truncate=true; an ordinary prompt.submit must not drop session history",
        ));
    }
    let int = |v: &Value, name: &str| -> Result<Option<u64>, Refusal> {
        if v.is_null() {
            return Ok(None);
        }
        v.as_u64().map(Some).ok_or_else(|| err(4004, &format!("{name} must be a non-negative integer")))
    };
    let ordinal = int(ordinal, "truncate_before_user_ordinal")?;
    let users = user_positions(rows);
    let stale = || err(4018, "that message is no longer in session history; reload the chat and try again");
    let by_row = match (int(row, "truncate_before_row_id")?, message.as_str()) {
        (Some(id), _) => Some(id),
        (None, Some(m)) => Some(m.parse::<u64>().map_err(|_| stale())?),
        (None, None) => None,
    };
    let target = match by_row {
        Some(id) => {
            let turn = users.iter().position(|&at| ids[at] == id).ok_or_else(stale)?;
            if ordinal.is_some_and(|o| o as usize != turn) {
                return Err(err(4030, "truncate_before_user_ordinal does not match the target row"));
            }
            turn
        }
        None => {
            let turn = ordinal.ok_or_else(stale)? as usize;
            if turn >= users.len() {
                return Err(stale());
            }
            turn
        }
    };
    if target == 0 && !rows.is_empty() && p["confirm_empty_truncate"].as_bool() != Some(true) {
        return Err(err(
            4028,
            "truncation would erase the entire session transcript; resubmit with confirm_empty_truncate=true if this is intended",
        ));
    }
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Value {
        json!({ "messages": [
            { "role": "user", "content": "one" },
            { "role": "assistant", "content": "a1" },
            { "role": "tool", "content": "t" },
            { "role": "user", "content": "two" },
            { "role": "assistant", "content": "a2" },
            { "role": "user", "content": "three" },
            { "role": "assistant", "content": "a3" },
        ] })
    }

    #[test]
    fn undo_cuts_the_last_user_turns_and_returns_their_text() {
        let rows = rows(&history());
        assert_eq!(rows.len(), 6, "tool rows are not rewind positions");
        let one = cut_last_users(&rows, 1).unwrap();
        assert_eq!((one.index, one.removed, one.text.as_str()), (4, 2, "three"));
        let two = cut_last_users(&rows, 2).unwrap();
        assert_eq!((two.index, two.removed, two.text.as_str()), (2, 4, "two"));
        let all = cut_last_users(&rows, 9).unwrap();
        assert_eq!((all.index, all.removed, all.text.as_str()), (0, 6, "one"));
        assert!(cut_last_users(&[], 1).is_none());
    }

    #[test]
    fn truncation_needs_confirmation_and_resolves_ordinal_and_row_id() {
        let rows = rows(&history());
        let ids = position_ids(&rows);
        assert_eq!(truncation(&json!({}), &rows, &ids).unwrap(), None);
        let unconfirmed = json!({ "truncate_before_user_ordinal": 1 });
        assert_eq!(truncation(&unconfirmed, &rows, &ids).err().unwrap().0, 4029);
        let by_ordinal = json!({ "truncate_before_user_ordinal": 1, "confirm_truncate": true });
        assert_eq!(truncation(&by_ordinal, &rows, &ids).unwrap(), Some(1));
        // With no id store, ids are 1-based row positions: the second user turn is row 3.
        let by_row = json!({ "truncate_before_row_id": 3, "confirm_truncate": true });
        assert_eq!(truncation(&by_row, &rows, &ids).unwrap(), Some(1));
        let by_message = json!({ "truncate_before_message_id": "5", "confirm_truncate": true });
        assert_eq!(truncation(&by_message, &rows, &ids).unwrap(), Some(2));
        let drift = json!({ "truncate_before_row_id": 3, "truncate_before_user_ordinal": 2, "confirm_truncate": true });
        assert_eq!(truncation(&drift, &rows, &ids).err().unwrap().0, 4030);
        let missing = json!({ "truncate_before_row_id": 2, "confirm_truncate": true });
        assert_eq!(truncation(&missing, &rows, &ids).err().unwrap().0, 4018, "an assistant row is not a user turn");
        let first = json!({ "truncate_before_user_ordinal": 0, "confirm_truncate": true });
        assert_eq!(truncation(&first, &rows, &ids).err().unwrap().0, 4028);
        let first_ok = json!({ "truncate_before_user_ordinal": 0, "confirm_truncate": true, "confirm_empty_truncate": true });
        assert_eq!(truncation(&first_ok, &rows, &ids).unwrap(), Some(0));
    }

    #[test]
    fn survivors_keep_their_ids() {
        let rows = rows(&history());
        let ids = position_ids(&rows);
        let (users, map) = survivors(&rows, &ids, 4);
        assert_eq!(users, [1, 3]);
        assert_eq!(map["3"], 3);
        assert!(map.get("5").is_none());
    }

    #[test]
    fn a_stale_id_from_an_undone_row_never_resolves_to_a_new_turn() {
        // Rows 1..4 were undone down to 2, then two new rows arrived with fresh ids 5 and 6.
        let rows = rows(&json!({ "messages": [
            { "role": "user", "content": "one" }, { "role": "assistant", "content": "a1" },
            { "role": "user", "content": "new" }, { "role": "assistant", "content": "a" },
        ] }));
        let ids = [1, 2, 5, 6];
        let stale = json!({ "truncate_before_row_id": 3, "confirm_truncate": true });
        assert_eq!(truncation(&stale, &rows, &ids).err().unwrap().0, 4018);
        let live = json!({ "truncate_before_row_id": 5, "confirm_truncate": true });
        assert_eq!(truncation(&live, &rows, &ids).unwrap(), Some(1));
        let (users, map) = survivors(&rows, &ids, 2);
        assert_eq!(users, [1]);
        assert_eq!(map, json!({ "1": 1, "2": 2 }));
    }
}
