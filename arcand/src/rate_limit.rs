//! Token-bucket rate limiter for arcand (BRO-223).
//!
//! Provides per-user, per-tier request throttling enforced in-process.
//! This is a defense-in-depth measure — even if a caller bypasses the
//! Next.js rate limiter, arcand will refuse excess requests.
//!
//! **Algorithm**: classic token bucket — tokens refill at a constant rate
//! up to a capacity equal to the per-minute limit. Each `POST /sessions/*/runs`
//! call consumes one token.
//!
//! **Bucket key**: `{tier}:{user_id_or_ip}` — one bucket per user per tier.
//!
//! **Multi-node note**: this implementation is single-node in-memory.
//! For arcand clusters (enterprise), swap `Mutex<HashMap>` for a
//! Redis-backed implementation (e.g., sliding window via `INCR`/`EXPIRE`).

use std::{collections::HashMap, sync::Mutex, time::Instant};

use crate::auth::Tier;

// ─── Rate limit config per tier ──────────────────────────────────────────────

/// Request rate limits for each capability tier.
#[derive(Debug, Clone, Copy)]
pub struct TierRateLimit {
    /// Maximum tokens in the bucket (burst capacity = per-minute limit).
    pub capacity: f64,
    /// Token refill rate in tokens-per-second.
    pub refill_rate: f64,
}

impl TierRateLimit {
    fn new(per_minute: f64) -> Self {
        Self {
            capacity: per_minute,
            refill_rate: per_minute / 60.0,
        }
    }

    /// Return the rate limit config for a given tier.
    pub fn for_tier(tier: &Tier) -> Self {
        match tier {
            Tier::Anonymous => Self::new(5.0),
            Tier::Free => Self::new(20.0),
            Tier::Pro => Self::new(60.0),
            Tier::Enterprise => Self::new(100.0),
        }
    }
}

// ─── Token bucket ────────────────────────────────────────────────────────────

struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_rate: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Attempt to consume one token.
    ///
    /// Returns `Ok(())` when a token was consumed successfully.
    /// Returns `Err(retry_after_secs)` when the bucket is empty.
    fn consume(&mut self) -> Result<(), u64> {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = Instant::now();

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let secs_needed = (1.0 - self.tokens) / self.refill_rate;
            Err(secs_needed.ceil() as u64)
        }
    }
}

// ─── Rate limiter ─────────────────────────────────────────────────────────────

/// In-memory token-bucket rate limiter.
///
/// Thread-safe via `Mutex<HashMap<String, TokenBucket>>`.
/// Use `Arc<RateLimiter>` to share across request handlers.
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when a request exceeds the rate limit.
#[derive(Debug)]
pub struct RateLimitError {
    /// How many seconds the caller should wait before retrying.
    pub retry_after: u64,
    /// The per-minute limit for this tier.
    pub limit_per_minute: u64,
    /// The tier that was rate-limited.
    pub tier: String,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Check the rate limit for a given (key, tier) pair.
    ///
    /// `key` should be `"{tier_name}:{user_id_or_ip}"`.
    ///
    /// Returns `Ok(())` if the request is allowed, or
    /// `Err(RateLimitError)` if the rate limit has been exceeded.
    pub fn check(&self, key: &str, tier: &Tier) -> Result<(), RateLimitError> {
        let config = TierRateLimit::for_tier(tier);

        let mut map = self.buckets.lock().expect("rate limiter mutex poisoned");
        let bucket = map
            .entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(config.capacity, config.refill_rate));

        bucket.consume().map_err(|retry_after| RateLimitError {
            retry_after,
            limit_per_minute: config.capacity as u64,
            tier: format!("{tier:?}").to_lowercase(),
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_limit_is_5_per_minute() {
        let cfg = TierRateLimit::for_tier(&Tier::Anonymous);
        assert_eq!(cfg.capacity as u64, 5);
    }

    #[test]
    fn free_limit_is_20_per_minute() {
        let cfg = TierRateLimit::for_tier(&Tier::Free);
        assert_eq!(cfg.capacity as u64, 20);
    }

    #[test]
    fn pro_limit_is_60_per_minute() {
        let cfg = TierRateLimit::for_tier(&Tier::Pro);
        assert_eq!(cfg.capacity as u64, 60);
    }

    #[test]
    fn enterprise_limit_is_100_per_minute() {
        let cfg = TierRateLimit::for_tier(&Tier::Enterprise);
        assert_eq!(cfg.capacity as u64, 100);
    }

    #[test]
    fn bucket_allows_up_to_capacity() {
        let limiter = RateLimiter::new();
        // Anonymous bucket = 5 tokens
        for i in 0..5 {
            assert!(
                limiter.check("anon:test-user", &Tier::Anonymous).is_ok(),
                "request {i} should succeed"
            );
        }
    }

    #[test]
    fn bucket_rejects_over_capacity() {
        let limiter = RateLimiter::new();
        // Consume all 5 tokens
        for _ in 0..5 {
            let _ = limiter.check("anon:burst-user", &Tier::Anonymous);
        }
        // 6th should be rate-limited
        let result = limiter.check("anon:burst-user", &Tier::Anonymous);
        assert!(result.is_err(), "6th request should be rate-limited");
        let err = result.unwrap_err();
        assert!(err.retry_after > 0);
        assert_eq!(err.limit_per_minute, 5);
        assert_eq!(err.tier, "anonymous");
    }

    #[test]
    fn different_users_have_independent_buckets() {
        let limiter = RateLimiter::new();
        // Exhaust user A
        for _ in 0..5 {
            let _ = limiter.check("anon:user-a", &Tier::Anonymous);
        }
        // User B should still be allowed
        assert!(limiter.check("anon:user-b", &Tier::Anonymous).is_ok());
    }

    #[test]
    fn pro_tier_allows_more_burst() {
        let limiter = RateLimiter::new();
        // Pro has 60 token capacity — should allow a 20-request burst without throttling
        for i in 0..20 {
            assert!(
                limiter.check("pro:pro-user", &Tier::Pro).is_ok(),
                "pro request {i} should succeed"
            );
        }
    }
}
