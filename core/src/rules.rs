use crate::telemetry::TelemetryReading;
use serde::{Deserialize, Serialize};

pub const LOW_BATTERY_THRESHOLD_V: f32 = 10.5;
pub const LOW_ALTITUDE_THRESHOLD_M: f32 = 50.0;
pub const HIGH_ATTITUDE_THRESHOLD_DEG: f32 = 45.0;

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum RuleKind {
    LowBattery,
    LowAltitude,
    ExtremeAttitude,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleViolation {
    pub kind: RuleKind,
    pub message: String,
}

pub fn evaluate(reading: &TelemetryReading) -> Vec<RuleViolation> {
    let mut violations = Vec::new();

    if reading.battery_voltage_v < LOW_BATTERY_THRESHOLD_V {
        violations.push(RuleViolation {
            kind: RuleKind::LowBattery,
            message: format!(
                "battery voltage {:.2}V is below the {:.2}V threshold",
                reading.battery_voltage_v, LOW_BATTERY_THRESHOLD_V
            ),
        });
    }

    if reading.altitude_m < LOW_ALTITUDE_THRESHOLD_M {
        violations.push(RuleViolation {
            kind: RuleKind::LowAltitude,
            message: format!(
                "altitude {:.1}m is below the {:.1}m threshold",
                reading.altitude_m, LOW_ALTITUDE_THRESHOLD_M
            ),
        });
    }

    if reading.attitude_deg > HIGH_ATTITUDE_THRESHOLD_DEG {
        violations.push(RuleViolation {
            kind: RuleKind::ExtremeAttitude,
            message: format!(
                "attitude {:.1}deg exceeds the {:.1}deg threshold",
                reading.attitude_deg, HIGH_ATTITUDE_THRESHOLD_DEG
            ),
        });
    }

    violations
}
