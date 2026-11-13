//! Raw observations from dedicated temperature sensors.

use std::time::Instant;

/// Last live temperature report received from one dedicated sensor.
#[derive(Debug, Clone, PartialEq)]
pub struct TemperatureSensorState {
    pub temperature: f64,
    pub received_at: Instant,
}
