use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::config::heating::{RemoteTemperatureConfig, RemoteTemperaturePolicy};
use crate::domain::Effect;
use crate::domain::action::Payload;
use crate::entities::temperature_sensor::TemperatureSensorState;

use super::EventProcessor;

const MIN_REMOTE_TEMPERATURE: f64 = 0.0;
const MAX_REMOTE_TEMPERATURE: f64 = 35.0;

fn source_state<'a>(
    source: &crate::config::heating::RemoteTemperatureSource,
    observations: &'a BTreeMap<String, TemperatureSensorState>,
    now: Instant,
) -> Option<(&'a TemperatureSensorState, Duration, bool)> {
    let observation = observations.get(&source.device)?;
    let age = now.checked_duration_since(observation.received_at)?;
    let fresh = age <= Duration::from_secs(u64::from(source.max_age_minutes) * 60);
    Some((observation, age, fresh))
}

pub(super) fn aggregate_remote_temperature(
    config: &RemoteTemperatureConfig,
    observations: &BTreeMap<String, TemperatureSensorState>,
    now: Instant,
) -> Option<f64> {
    let fresh = |source: &crate::config::heating::RemoteTemperatureSource| {
        source_state(source, observations, now)
            .and_then(|(observation, _, fresh)| fresh.then_some(observation))
    };

    match config.policy {
        RemoteTemperaturePolicy::Fallback => config
            .sources
            .iter()
            .find_map(|source| fresh(source).map(|observation| observation.temperature)),
        RemoteTemperaturePolicy::Freshest => {
            let mut newest: Option<&TemperatureSensorState> = None;
            for source in &config.sources {
                let Some(observation) = fresh(source) else {
                    continue;
                };
                if newest.is_none_or(|current| observation.received_at > current.received_at) {
                    newest = Some(observation);
                }
            }
            newest.map(|observation| observation.temperature)
        }
        RemoteTemperaturePolicy::Min => config
            .sources
            .iter()
            .filter_map(fresh)
            .map(|observation| observation.temperature)
            .reduce(f64::min),
        RemoteTemperaturePolicy::Max => config
            .sources
            .iter()
            .filter_map(fresh)
            .map(|observation| observation.temperature)
            .reduce(f64::max),
        RemoteTemperaturePolicy::Average => {
            let (sum, count) = config
                .sources
                .iter()
                .filter_map(fresh)
                .map(|observation| observation.temperature)
                .fold((0.0, 0_u32), |(sum, count), temperature| {
                    (sum + temperature, count + 1)
                });
            (count > 0).then_some(sum / f64::from(count))
        }
    }
}

impl EventProcessor {
    pub(crate) fn remote_temperature_aggregate(
        &self,
        config: &RemoteTemperatureConfig,
        now: Instant,
    ) -> Option<f64> {
        aggregate_remote_temperature(config, &self.world.temperature_sensors, now)
    }

    pub(crate) fn remote_temperature_source_state<'a>(
        &'a self,
        source: &crate::config::heating::RemoteTemperatureSource,
        now: Instant,
    ) -> Option<(&'a TemperatureSensorState, Duration, bool)> {
        source_state(source, &self.world.temperature_sensors, now)
    }

    pub(super) fn handle_temperature_sensor_state(
        &mut self,
        heating_config: &crate::config::heating::HeatingConfig,
        device: &str,
        temperature: f64,
        ts: Instant,
    ) -> Vec<Effect> {
        self.world.temperature_sensors.insert(
            device.to_string(),
            TemperatureSensorState {
                temperature,
                received_at: ts,
            },
        );

        let mut effects = Vec::new();
        for zone in &heating_config.zones {
            for trv in &zone.trvs {
                let Some(remote) = &trv.remote_temperature else {
                    continue;
                };
                if !remote.sources.iter().any(|source| source.device == device) {
                    continue;
                }
                let Some(aggregate) =
                    aggregate_remote_temperature(remote, &self.world.temperature_sensors, ts)
                else {
                    continue;
                };
                if !aggregate.is_finite()
                    || !(MIN_REMOTE_TEMPERATURE..=MAX_REMOTE_TEMPERATURE).contains(&aggregate)
                {
                    tracing::warn!(
                        trv = %trv.device,
                        sensor = device,
                        aggregate,
                        "remote temperature aggregate outside Bosch range; command suppressed"
                    );
                    continue;
                }
                let trv_idx = self
                    .topology
                    .device_idx(&trv.device)
                    .expect("validated heating TRV must exist in topology");
                effects.push(Effect::PublishDeviceSet {
                    device: trv_idx,
                    payload: Payload::RemoteTemperature {
                        remote_temperature: aggregate,
                    },
                });
            }
        }
        effects
    }
}
