use crate::rules::RuleViolation;
use std::collections::HashMap;

pub struct AlertDedupe {
    ttl_ms: u64,
    last_emitted: HashMap<String, u64>,
}

impl AlertDedupe {
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            ttl_ms,
            last_emitted: HashMap::new(),
        }
    }

    pub fn should_emit(&mut self, device_id: &str, violation: &RuleViolation, ts: u64) -> bool {
        let key = format!("{}:{:?}", device_id, violation.kind);
        let last = self.last_emitted.get(&key).copied().unwrap_or(0);
        if ts.saturating_sub(last) > self.ttl_ms {
            self.last_emitted.insert(key, ts);
            true
        } else {
            false
        }
    }
}
