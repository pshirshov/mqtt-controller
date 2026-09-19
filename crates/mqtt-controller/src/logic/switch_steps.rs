use std::time::{Duration, Instant};

use crate::domain::{Effect, action::Payload};
use crate::entities::light::LightTarget;
use crate::entities::light_zone::{LightZoneTarget, SwitchCycle};
use crate::tass::Owner;
use crate::topology::ResolvedSwitchStep;

use super::EventProcessor;

pub(super) enum SwitchAction {
    Cycle,
    Toggle,
    ToggleCycle,
}

impl EventProcessor {
    fn active_switch_steps(
        &mut self,
        room_name: &str,
    ) -> Option<(String, Vec<ResolvedSwitchStep>)> {
        if self
            .topology
            .room_by_name(room_name)?
            .switch_steps
            .is_empty()
        {
            return None;
        }
        let sun = self.sun_times();
        let room = self.topology.room_by_name(room_name).expect("known room");
        let (slot, _) = room.scenes.slot_for_time(
            self.clock.local_hour(),
            self.clock.local_minute(),
            sun.as_ref(),
        )?;
        room.switch_steps
            .get(slot)
            .map(|steps| (slot.clone(), steps.clone()))
    }

    pub(super) fn execute_switch_steps(
        &mut self,
        room_name: &str,
        action: SwitchAction,
        ts: Instant,
    ) -> Option<Vec<Effect>> {
        let (slot, steps) = self.active_switch_steps(room_name)?;
        let zone = self.world.light_zone(room_name);
        let is_on = zone.is_on()
            && !(zone.target.is_actionable()
                && matches!(zone.target.value(), Some(LightZoneTarget::Off)));
        let turn_off = is_on
            && match action {
                SwitchAction::Cycle => false,
                SwitchAction::Toggle => true,
                SwitchAction::ToggleCycle => !zone.last_press_at.is_some_and(|last| {
                    ts.duration_since(last)
                        < Duration::from_secs_f64(self.defaults.cycle_window_seconds)
                }),
            };
        if turn_off {
            return Some(self.execute_turn_off_room(room_name, ts));
        }
        let index = if is_on && zone.target_is_on() {
            zone.switch_cycle
                .as_ref()
                .filter(|cycle| cycle.slot == slot)
                .map_or(0, |cycle| (cycle.step + 1) % steps.len())
        } else {
            0
        };
        let step = &steps[index];
        let room = self.topology.room_by_name(room_name).expect("known room");
        let members = room.light_members.clone();
        let off_transition = room.off_transition_seconds;
        let mut effects = Vec::with_capacity(members.len());
        for member in members {
            let selected = step.lights.contains(&member);
            let owner = if self.light_has_off_only_claim(member.device) {
                Owner::Motion
            } else {
                Owner::User
            };
            let target = if selected {
                LightTarget::On {
                    brightness: step.scene.brightness,
                    color_temp: step.scene.color_temp,
                }
            } else {
                LightTarget::Off
            };
            let name = self.topology.device_name(member.device).to_string();
            self.world
                .light(&name)
                .target
                .set_and_command(target, owner, ts);
            effects.push(Effect::PublishLightSet {
                light: member,
                payload: if selected {
                    Payload::light_on(&step.scene)
                } else {
                    Payload::state_off(off_transition)
                },
            });
        }
        let owner = self.resolve_zone_owner(room_name, Owner::User);
        let zone = self.world.light_zone(room_name);
        zone.target.set_and_command(
            LightZoneTarget::On {
                scene_id: step.scene.id,
                cycle_idx: index,
            },
            owner,
            ts,
        );
        zone.switch_cycle = Some(SwitchCycle {
            slot: slot.clone(),
            step: index,
        });
        zone.last_press_at = Some(ts);
        tracing::info!(room = room_name, %slot, step = index, scene = step.scene.id, "switch step applied");
        Some(effects)
    }

    pub(super) fn switch_step_is_confirmed(&self, room_name: &str) -> bool {
        let Some(zone) = self.world.light_zones.get(room_name) else {
            return true;
        };
        let Some(cycle) = &zone.switch_cycle else {
            return true;
        };
        let room = self.topology.room_by_name(room_name).expect("known room");
        let step = &room.switch_steps[&cycle.slot][cycle.step];
        let commanded = zone
            .target
            .since()
            .expect("switch step has a target timestamp");
        room.light_members.iter().all(|member| {
            let Some(light) = self
                .world
                .lights
                .get(self.topology.device_name(member.device))
            else {
                return false;
            };
            let Some(expected) = light.target.value() else {
                return false;
            };
            let selected = step.lights.contains(member);
            matches!(expected, LightTarget::On { .. }) == selected
                && light
                    .actual
                    .value()
                    .is_some_and(|actual| expected.matches(actual))
                && light.actual.since().is_some_and(|since| since >= commanded)
        })
    }

    pub(super) fn execute_switch_brightness(
        &mut self,
        room_name: &str,
        payload: Payload,
        ts: Instant,
    ) -> Option<Vec<Effect>> {
        // Keep the active selection across schedule boundaries until an ON
        // press chooses a new step; release must reach the lights we moved.
        let cycle = self
            .world
            .light_zones
            .get(room_name)?
            .switch_cycle
            .as_ref()?;
        let room = self.topology.room_by_name(room_name).expect("known room");
        let lights = room.switch_steps[&cycle.slot][cycle.step].lights.clone();
        let mut effects = Vec::with_capacity(lights.len());
        for endpoint in lights {
            let owner = if self.light_has_off_only_claim(endpoint.device) {
                Owner::Motion
            } else {
                Owner::User
            };
            let name = self.topology.device_name(endpoint.device).to_string();
            self.world.light(&name).target.set_and_command(
                LightTarget::On {
                    brightness: None,
                    color_temp: None,
                },
                owner,
                ts,
            );
            effects.push(Effect::PublishLightSet {
                light: endpoint,
                payload: payload.clone(),
            });
        }
        let owner = self.resolve_zone_owner(room_name, Owner::User);
        self.world
            .light_zone(room_name)
            .target
            .reassign_owner(owner, ts);
        Some(effects)
    }

    pub(super) fn switch_step_is_off(&self, room_name: &str) -> bool {
        let Some(zone) = self.world.light_zones.get(room_name) else {
            return false;
        };
        let Some(cycle) = &zone.switch_cycle else {
            return false;
        };
        let room = self.topology.room_by_name(room_name).expect("known room");
        let step = &room.switch_steps[&cycle.slot][cycle.step];
        let since = zone
            .target
            .since()
            .expect("switch step has a target timestamp");
        // Excluded members were already OFF, so they need not report again
        // when the selected members are subsequently switched off externally.
        step.lights.iter().all(|member| {
            self.world
                .lights
                .get(self.topology.device_name(member.device))
                .is_some_and(|light| {
                    light.actual.value().is_some_and(|actual| !actual.on)
                        && light
                            .actual
                            .since()
                            .is_some_and(|observed| observed >= since)
                })
        })
    }
}
