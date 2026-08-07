use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A simple in-memory rate limiter to prevent authentication abuse.
///
/// It tracks failures per key (e.g., issuer or IP) and enforces a
/// cooldown period or rejection after a threshold is reached.
pub struct RateLimiter {
    state: Arc<Mutex<RateLimitState>>,
    config: RateLimitConfig,
}

#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub max_failures: usize,
    pub window: Duration,
    pub block_duration: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_failures: 10,
            window: Duration::from_secs(60),
            block_duration: Duration::from_secs(300),
        }
    }
}

struct RateLimitState {
    counters: HashMap<String, Entry>,
}

struct Entry {
    failures: Vec<Instant>,
    blocked_until: Option<Instant>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimitState {
                counters: HashMap::new(),
            })),
            config,
        }
    }

    /// Check if the given key is currently allowed to attempt authentication.
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();

        let entry = state
            .counters
            .entry(key.to_string())
            .or_insert_with(|| Entry {
                failures: Vec::new(),
                blocked_until: None,
            });

        // 1. Check if blocked
        if let Some(blocked_until) = entry.blocked_until {
            if now < blocked_until {
                return Err(blocked_until.duration_since(now));
            } else {
                // Block expired, reset failures for a clean slate
                entry.blocked_until = None;
                entry.failures.clear();
            }
        }

        // 2. Clean up old failures outside the window
        let window_start = now.checked_sub(self.config.window).unwrap_or(now);
        entry.failures.retain(|&t| t > window_start);

        // 3. Check threshold
        if entry.failures.len() >= self.config.max_failures {
            let blocked_until = now + self.config.block_duration;
            entry.blocked_until = Some(blocked_until);
            return Err(self.config.block_duration);
        }

        Ok(())
    }

    /// Record a failed authentication attempt for the given key.
    pub fn record_failure(&self, key: &str) {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();

        let entry = state
            .counters
            .entry(key.to_string())
            .or_insert_with(|| Entry {
                failures: Vec::new(),
                blocked_until: None,
            });

        entry.failures.push(now);

        // If threshold reached just now, block immediately
        if entry.failures.len() >= self.config.max_failures {
            entry.blocked_until = Some(now + self.config.block_duration);
        }
    }

    /// Reset counters for a key (e.g., after a successful admin review or auth).
    pub fn reset(&self, key: &str) {
        let mut state = self.state.lock().unwrap();
        state.counters.remove(key);
    }
}
