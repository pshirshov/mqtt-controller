use std::time::Duration;

use crate::logic::EventProcessor;
use crate::settings::ValveBoost;

impl EventProcessor {
    pub fn active_boost(&self, device: &str) -> Option<&ValveBoost> {
        self.settings.boosts.get(device).filter(|boost| {
            let deadline = self
                .boost_deadlines
                .get(device)
                .expect("boost must have a monotonic deadline");
            self.clock.now() < *deadline && self.clock.epoch_millis() < boost.ends_at_epoch_ms
        })
    }

    pub(crate) fn effective_heat_demand_enabled(&self, device: &str) -> bool {
        self.heat_demand_enabled(device) || self.active_boost(device).is_some()
    }

    pub(crate) fn boost_remaining_ms(&self, device: &str) -> Option<u64> {
        self.active_boost(device).map(|boost| {
            let deadline = self.boost_deadlines[device];
            (deadline
                .saturating_duration_since(self.clock.now())
                .as_millis() as u64)
                .min(
                    boost
                        .ends_at_epoch_ms
                        .saturating_sub(self.clock.epoch_millis()),
                )
        })
    }

    pub(crate) fn apply_valve_boost(&mut self, device: &str, boost: Option<ValveBoost>) {
        if let Some(boost) = boost {
            // Editing the temperature must not reset the monotonic timer, even if wall time changed.
            if self.settings.boosts.get(device).map(|b| b.ends_at_epoch_ms)
                != Some(boost.ends_at_epoch_ms)
            {
                self.boost_deadlines.insert(
                    device.into(),
                    self.clock.now()
                        + Duration::from_millis(
                            boost
                                .ends_at_epoch_ms
                                .saturating_sub(self.clock.epoch_millis()),
                        ),
                );
            }
            self.settings.boosts.insert(device.into(), boost);
            tracing::info!(
                device,
                temperature = boost.temperature,
                ends_at_epoch_ms = boost.ends_at_epoch_ms,
                "valve boost saved"
            );
        } else {
            self.settings.boosts.remove(device);
            self.boost_deadlines.remove(device);
            tracing::info!(device, "valve boost cancelled");
        }
    }

    pub(crate) fn restore_boost_deadlines(&mut self) {
        let now = self.clock.now();
        let epoch_ms = self.clock.epoch_millis();
        self.boost_deadlines = self
            .settings
            .boosts
            .iter()
            .map(|(device, boost)| {
                (
                    device.clone(),
                    now + Duration::from_millis(boost.ends_at_epoch_ms.saturating_sub(epoch_ms)),
                )
            })
            .collect();
    }
}
