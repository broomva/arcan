//! Cost estimation helpers shared across the arcan CLI surfaces.
//!
//! Originally a private helper in the shell REPL (`src/shell.rs`,
//! BRO-364), promoted to the library so the `arcan agent test`
//! live-LLM path (BRO-1008) can report an estimated spend per run
//! without duplicating the pricing table. Both surfaces deliberately
//! share one table so an operator sees consistent numbers between
//! `arcan shell` and `arcan agent test --live`.

/// Estimate cost in USD for a model call based on token usage and model name.
///
/// Uses published Claude pricing (per million tokens):
/// - Opus: $15 input, $75 output
/// - Sonnet: $3 input, $15 output
/// - Haiku: $0.25 input, $1.25 output
///
/// Unknown models default to Sonnet pricing — the mid-tier estimate is
/// the least-surprising fallback for budget displays.
pub fn estimate_cost(input_tokens: u64, output_tokens: u64, model: &str) -> f64 {
    let (input_rate, output_rate) = match model {
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("sonnet") => (3.0, 15.0),
        m if m.contains("haiku") => (0.25, 1.25),
        _ => (3.0, 15.0), // default to sonnet pricing
    };
    (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- BRO-364: estimate_cost tests (moved from src/shell.rs) ---

    #[test]
    fn test_estimate_cost_opus() {
        let cost = estimate_cost(1_000_000, 1_000_000, "claude-opus-4-20250514");
        // $15 input + $75 output = $90
        assert!((cost - 90.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_sonnet() {
        let cost = estimate_cost(1_000_000, 1_000_000, "claude-sonnet-4-20250514");
        // $3 input + $15 output = $18
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_haiku() {
        let cost = estimate_cost(1_000_000, 1_000_000, "claude-haiku-4-20250514");
        // $0.25 input + $1.25 output = $1.50
        assert!((cost - 1.50).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_unknown_defaults_to_sonnet() {
        let cost = estimate_cost(1_000_000, 1_000_000, "some-unknown-model");
        // Should use sonnet pricing: $3 + $15 = $18
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_zero_tokens() {
        let cost = estimate_cost(0, 0, "claude-sonnet-4-20250514");
        assert!((cost).abs() < f64::EPSILON);
    }

    #[test]
    fn test_estimate_cost_typical_turn() {
        // A typical turn: 5000 input, 1000 output, sonnet
        let cost = estimate_cost(5000, 1000, "claude-sonnet-4-20250514");
        // (5000 * 3 + 1000 * 15) / 1_000_000 = (15000 + 15000) / 1_000_000 = 0.03
        assert!((cost - 0.03).abs() < 0.001);
    }
}
