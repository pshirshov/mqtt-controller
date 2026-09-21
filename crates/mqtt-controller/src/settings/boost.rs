use super::SettingsRepository;
use crate::logic::EventProcessor;
use crate::logic::heating::{MAX_SETPOINT, MIN_SETPOINT};

pub const BOOST_DURATIONS_MINUTES: [u16; 7] = [30, 60, 90, 120, 180, 240, 360];
const MILLIS_PER_MINUTE: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValveBoost {
    pub temperature: f64,
    pub ends_at_epoch_ms: u64,
}

pub enum BoostChange {
    Start {
        duration_minutes: u16,
        temperature: f64,
    },
    Target {
        temperature: f64,
    },
    Cancel,
}

pub(super) fn validate_temperature(temperature: f64) -> Result<(), String> {
    if !temperature.is_finite()
        || !(MIN_SETPOINT..=MAX_SETPOINT).contains(&temperature)
        || (temperature * 2.0).fract() != 0.0
    {
        return Err("Boost target must be 5–30°C in 0.5°C steps".into());
    }
    Ok(())
}

pub async fn change_valve_boost(
    processor: &mut EventProcessor,
    repository: &impl SettingsRepository,
    device: &str,
    change: BoostChange,
) -> Result<(), String> {
    processor.validate_heat_demand_device(device)?;
    let boost = match change {
        BoostChange::Start {
            duration_minutes,
            temperature,
        } => {
            validate_temperature(temperature)?;
            if !BOOST_DURATIONS_MINUTES.contains(&duration_minutes) {
                return Err(
                    "Boost duration must be 30, 60, 90, 120, 180, 240 or 360 minutes".into(),
                );
            }
            if processor.active_boost(device).is_some() {
                return Err("Boost is already active; edit its target or cancel it first".into());
            }
            let ends_at_epoch_ms = processor
                .clock
                .epoch_millis()
                .checked_add(u64::from(duration_minutes) * MILLIS_PER_MINUTE)
                .filter(|end| *end <= i64::MAX as u64)
                .ok_or("Boost deadline is outside the supported timestamp range")?;
            Some(ValveBoost {
                temperature,
                ends_at_epoch_ms,
            })
        }
        BoostChange::Target { temperature } => {
            validate_temperature(temperature)?;
            let current = processor
                .active_boost(device)
                .ok_or("Boost is no longer active")?;
            Some(ValveBoost {
                temperature,
                ..*current
            })
        }
        BoostChange::Cancel => None,
    };
    repository
        .set_valve_boost(device, boost.as_ref())
        .await
        .map_err(|error| format!("Could not save boost for {device}: {error}"))?;
    processor.apply_valve_boost(device, boost);
    Ok(())
}
