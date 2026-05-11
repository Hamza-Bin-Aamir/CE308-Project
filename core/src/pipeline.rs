use crate::telemetry::TelemetryReading;
use serde::{Deserialize, Serialize};

use crate::rules;
use crate::dedupe::AlertDedupe;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertOutcome {
    pub violations: Vec<rules::RuleViolation>,
    pub emitted_alerts: Vec<rules::RuleViolation>,
}

pub fn process_reading(reading: &TelemetryReading, dedupe: &mut AlertDedupe) -> AlertOutcome {
    let violations = rules::evaluate(reading);
    let mut emitted_alerts = Vec::new();

    for violation in &violations {
        if dedupe.should_emit(&reading.device_id, violation, reading.timestamp_ms) {
            emitted_alerts.push(violation.clone());
        }
    }

    AlertOutcome {
        violations,
        emitted_alerts,
    }
}
