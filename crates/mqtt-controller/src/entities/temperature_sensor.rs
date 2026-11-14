//! Raw observations from dedicated temperature sensors.

use std::time::Instant;

/// Last temperature observation with an established observation time.
#[derive(Debug, Clone, PartialEq)]
pub struct TemperatureSensorState {
    pub temperature: f64,
    pub received_at: Instant,
}
