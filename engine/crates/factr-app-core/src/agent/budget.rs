//! Optional hard budget stop (`budget.max_cost_usd`, `budget.max_tokens`; both unset by default).
//!
//! Checked at the head of each provider request in the turn loops. With nothing configured the check
//! returns immediately without touching usage, so the default path is unchanged.

use super::Agent;

/// The stop message when a limit is reached. `cost` is `None` for routes that cannot be priced; a cost
/// limit is then not enforced (the token limit still is).
pub(super) fn budget_stop_message(
    max_cost_usd: Option<f64>,
    max_tokens: Option<u64>,
    used_tokens: u64,
    cost_usd: Option<f64>,
) -> Option<String> {
    if let (Some(max), Some(cost)) = (max_cost_usd, cost_usd)
        && cost >= max
    {
        return Some(format!(
            "Budget reached: this session has used about ${cost:.2} of its ${max:.2} limit (budget.max_cost_usd). \
             Raise or clear the limit in Settings to continue."
        ));
    }
    if let Some(max) = max_tokens
        && used_tokens >= max
    {
        return Some(format!(
            "Budget reached: this session has used {used_tokens} tokens of its {max} token limit (budget.max_tokens). \
             Raise or clear the limit in Settings to continue."
        ));
    }
    None
}

impl Agent {
    /// `Some(message)` when the configured session budget is used up.
    pub(super) fn budget_stop(&self) -> Option<String> {
        let budget = factr_base::factr_config::current().budget.clone();
        if budget.max_cost_usd.is_none() && budget.max_tokens.is_none() {
            return None;
        }
        let totals = self.session.token_usage_totals();
        let cost = budget.max_cost_usd.and_then(|_| {
            let key = factr_base::provider_activity::source_key_for_provider_label(self.provider.name(), None);
            factr_base::provider::pricing::metered_usage_cost_usd(
                &key,
                &self.provider.model(),
                totals.input_tokens,
                totals.output_tokens,
                totals.cache_read_input_tokens,
                totals.cache_creation_input_tokens,
            )
        });
        budget_stop_message(budget.max_cost_usd, budget.max_tokens, totals.input_tokens + totals.output_tokens, cost)
    }
}

#[cfg(test)]
mod tests {
    use super::budget_stop_message;

    #[test]
    fn nothing_configured_never_stops() {
        assert_eq!(budget_stop_message(None, None, u64::MAX, Some(1e9)), None);
    }

    #[test]
    fn a_cost_limit_stops_at_or_above_it_and_is_skipped_for_unpriced_routes() {
        assert!(budget_stop_message(Some(1.0), None, 0, Some(1.0)).unwrap().contains("$1.00"));
        assert_eq!(budget_stop_message(Some(1.0), None, 0, Some(0.99)), None);
        assert_eq!(budget_stop_message(Some(1.0), None, 10_000_000, None), None, "unpriced: not enforced");
    }

    #[test]
    fn a_token_limit_applies_even_when_unpriced() {
        assert!(budget_stop_message(None, Some(1000), 1000, None).unwrap().contains("1000 tokens"));
        assert_eq!(budget_stop_message(None, Some(1000), 999, None), None);
    }
}
