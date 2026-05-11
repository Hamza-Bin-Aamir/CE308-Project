use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryReading {
    pub device_id: String,
    pub timestamp_ms: u64,
    pub battery_voltage_v: f32,
    pub altitude_m: f32,
    pub attitude_deg: f32,
    pub gps_lat: f64,
    pub gps_lon: f64,
}
