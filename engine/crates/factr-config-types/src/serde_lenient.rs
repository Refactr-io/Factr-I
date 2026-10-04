//! Lenient serde helper for string-valued configuration enums.
//!
//! A single unrecognized enum value must not abort parsing of the *entire*
//! `config.toml` (issue #689): the one field that is wrong degrades instead.

use serde::Deserialize;

/// `Option` enum deserializer: an unrecognized value becomes `None`
/// ("not configured") rather than a config-wide parse failure.
pub(crate) fn lenient_optional_enum<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let value = serde::de::value::StrDeserializer::<serde::de::value::Error>::new(raw.as_str());
    Ok(T::deserialize(value).ok())
}
