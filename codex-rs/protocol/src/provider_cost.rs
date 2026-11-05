//! Running total of provider-reported request costs for this process.
//!
//! cortecs.ai reports the price of each Responses API request in `usage.cost`, in millionths of
//! the account currency (EUR by default). The model client records every completed or
//! incomplete response here so the TUI can show what the session has cost so far. The total
//! spans all threads in the process (sub-agents, compaction and review requests included).

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

static TOTAL_MICROS: AtomicU64 = AtomicU64::new(0);
static RECORDED: AtomicBool = AtomicBool::new(false);

/// Record the `cost` field of a Responses API `usage` object, if present.
pub fn record_usage_cost(usage: &serde_json::Value) {
    let Some(cost) = usage.get("cost").and_then(serde_json::Value::as_f64) else {
        return;
    };
    if !cost.is_finite() || cost < 0.0 {
        return;
    }
    TOTAL_MICROS.fetch_add(cost.round() as u64, Ordering::Relaxed);
    RECORDED.store(true, Ordering::Relaxed);
}

/// Total provider-reported cost in millionths of the account currency, or `None` when the
/// provider has not reported any cost in this process.
pub fn total_cost_micros() -> Option<u64> {
    RECORDED
        .load(Ordering::Relaxed)
        .then(|| TOTAL_MICROS.load(Ordering::Relaxed))
}

/// Format a micro-unit amount as euros, with enough precision for cheap requests.
pub fn format_eur_micros(micros: u64) -> String {
    let euros = micros as f64 / 1_000_000.0;
    if euros >= 1.0 {
        format!("€{euros:.2}")
    } else {
        format!("€{euros:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn formats_amounts() {
        assert_eq!(format_eur_micros(153), "€0.0002");
        assert_eq!(format_eur_micros(12_345), "€0.0123");
        assert_eq!(format_eur_micros(2_500_000), "€2.50");
    }

    #[test]
    fn records_reported_cost() {
        let before = total_cost_micros().unwrap_or(0);
        record_usage_cost(&serde_json::json!({"input_tokens": 1, "cost": 153.0}));
        record_usage_cost(&serde_json::json!({"input_tokens": 1}));
        assert_eq!(total_cost_micros(), Some(before + 153));
    }
}
