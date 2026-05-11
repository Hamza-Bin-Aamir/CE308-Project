use ce308_cc::{dedupe::AlertDedupe, pipeline, rules::RuleKind, telemetry::TelemetryReading};

#[test]
fn detects_low_battery_and_low_altitude() {
    let reading = TelemetryReading {
        device_id: "uav-01".to_string(),
        timestamp_ms: 1000,
        battery_voltage_v: 9.8,
        altitude_m: 42.0,
        attitude_deg: 12.0,
        gps_lat: 33.0,
        gps_lon: 73.0,
    };

    let mut dedupe = AlertDedupe::new(60_000);
    let outcome = pipeline::process_reading(&reading, &mut dedupe);

    assert_eq!(outcome.violations.len(), 2);
    assert!(outcome
        .violations
        .iter()
        .any(|violation| violation.kind == RuleKind::LowBattery));
    assert!(outcome
        .violations
        .iter()
        .any(|violation| violation.kind == RuleKind::LowAltitude));
    assert_eq!(outcome.emitted_alerts.len(), 2);
}

#[test]
fn suppresses_duplicate_alerts_within_ttl() {
    let reading = TelemetryReading {
        device_id: "uav-01".to_string(),
        timestamp_ms: 1000,
        battery_voltage_v: 9.8,
        altitude_m: 42.0,
        attitude_deg: 12.0,
        gps_lat: 33.0,
        gps_lon: 73.0,
    };

    let mut dedupe = AlertDedupe::new(60_000);
    let first = pipeline::process_reading(&reading, &mut dedupe);

    let second_reading = TelemetryReading {
        timestamp_ms: 20_000,
        ..reading
    };
    let second = pipeline::process_reading(&second_reading, &mut dedupe);

    assert_eq!(first.emitted_alerts.len(), 2);
    assert!(second.emitted_alerts.is_empty());
}
