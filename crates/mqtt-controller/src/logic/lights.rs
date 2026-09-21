//! Light zone logic: scene cycling, toggle, brightness, group state echo,
//! and descendant propagation.
//!
//! State access goes through [`WorldState`] / [`LightZoneEntity`] TASS
//! entities with target/actual separation.

use std::time::{Duration, Instant};

use crate::domain::Effect;
use crate::domain::action::Payload;
use crate::entities::light::LightTarget;
use crate::entities::light_zone::{LightZoneActual, LightZoneTarget};
use crate::tass::{Owner, TassActual, TargetPhase};
use crate::topology::{RoomIdx, RoomName};

use super::EventProcessor;
use super::switch_steps::SwitchAction;

impl EventProcessor {
    pub(super) fn off_only_session_live(&self, room_name: &str) -> bool {
        self.topology.room_by_name(room_name).is_some_and(|room| {
            !room.light_members.is_empty()
                && room
                    .light_members
                    .iter()
                    .all(|light| self.light_has_off_only_claim(light.device))
        })
    }

    /// Pick the owner for a write driven by a user-originated action (a
    /// physical button press, a web UI click, etc). Returns
    /// `Owner::Motion` only when [`Self::off_only_session_live`]
    /// reports a live session — the one case where user actions must
    /// not be allowed to defeat the scheduled motion-off. Every other
    /// case returns `default_owner` unchanged so the lux/cooldown gate
    /// suppression is honoured and zombie claims don't backdoor it.
    pub(super) fn resolve_zone_owner(&self, room_name: &str, default_owner: Owner) -> Owner {
        if self.off_only_session_live(room_name) {
            Owner::Motion
        } else {
            default_owner
        }
    }
    /// `SceneCycle` effect -- wall switch on-button behavior. Pure scene
    /// cycle: every press advances the cycle unconditionally, no time
    /// component, no toggle-off. Per-slot switch steps override the
    /// ordinary whole-group scene cycle when configured.
    pub(super) fn execute_scene_cycle(&mut self, room_name: &str, ts: Instant) -> Vec<Effect> {
        if let Some(effects) = self.execute_switch_steps(room_name, SwitchAction::Cycle, ts) {
            return effects;
        }
        self.execute_group_scene_cycle(room_name, ts)
    }

    /// Cycle the whole room group, bypassing per-light switch steps.
    pub(super) fn execute_group_scene_cycle(
        &mut self,
        room_name: &str,
        ts: Instant,
    ) -> Vec<Effect> {
        let scenes_for_now = self.scenes_for_room(room_name);
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();

        if scenes_for_now.is_empty() {
            return Vec::new();
        }
        let n = scenes_for_now.len();

        let zone = self.world.light_zone(room_name);
        let (next_idx, branch) = if !zone.is_on() {
            (0, "start cycle (off)")
        } else if zone.switch_cycle.is_some() {
            let current_scene = match zone.target.value() {
                Some(LightZoneTarget::On { scene_id, .. }) => Some(*scene_id),
                _ => None,
            };
            let next = current_scene
                .and_then(|scene_id| scenes_for_now.iter().position(|id| *id == scene_id))
                .map_or(0, |index| (index + 1) % n);
            (next, "cycle advance (leaving switch steps)")
        } else {
            ((zone.cycle_idx() + 1) % n, "cycle advance")
        };
        let prev_idx = zone.cycle_idx();
        let next_scene = scenes_for_now[next_idx];
        tracing::info!(
            room = room_name,
            group = %group_name,
            scene = next_scene,
            cycle_idx_from = prev_idx,
            cycle_idx_to = next_idx,
            cycle_len = n,
            branch,
            "scene_cycle → scene_recall"
        );
        let effect = Effect::PublishGroupSet {
            room: room_idx,
            payload: Payload::scene_recall(next_scene),
        };
        let owner = self.resolve_zone_owner(room_name, Owner::User);
        self.write_after_on(room_name, ts, next_idx, next_scene, owner);
        self.world.light_zone(room_name).switch_cycle = None;
        self.propagate_to_descendants(room_name, true, ts);
        vec![effect]
    }

    /// `SceneToggle` effect -- pure on/off toggle. If room is off, turn
    /// on with the first scene in the active slot. If room is on, turn
    /// off. No cycle window, no scene advancement. Designed for buttons
    /// that use hardware double-tap for scene cycling.
    pub(super) fn execute_scene_toggle(&mut self, room_name: &str, ts: Instant) -> Vec<Effect> {
        if let Some(effects) = self.execute_switch_steps(room_name, SwitchAction::Toggle, ts) {
            return effects;
        }
        let scenes_for_now = self.scenes_for_room(room_name);
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();
        let off_transition = room.off_transition_seconds;

        if scenes_for_now.is_empty() {
            return Vec::new();
        }

        let zone = self.world.light_zone(room_name);
        let is_on = zone.is_on();

        if !is_on {
            let first = scenes_for_now[0];
            tracing::info!(
                room = room_name,
                group = %group_name,
                scene = first,
                branch = "on (was off)",
                "scene_toggle → scene_recall"
            );
            let effect = Effect::PublishGroupSet {
                room: room_idx,
                payload: Payload::scene_recall(first),
            };
            let owner = self.resolve_zone_owner(room_name, Owner::User);
            self.write_after_on(room_name, ts, 0, first, owner);
            self.propagate_to_descendants(room_name, true, ts);
            vec![effect]
        } else {
            tracing::info!(
                room = room_name,
                group = %group_name,
                transition = off_transition,
                branch = "off (was on)",
                "scene_toggle → state OFF"
            );
            let mut out = Vec::new();
            let owner = self.resolve_zone_owner(room_name, Owner::User);
            self.publish_off(room_name, room_idx, off_transition, ts, &mut out, owner);
            out
        }
    }

    /// `SceneToggleCycle` effect -- tap button three-branch behavior:
    /// 1. If room is off -> turn on with first scene
    /// 2. If within cycle window -> advance to next scene
    /// 3. If outside cycle window -> turn off
    pub(super) fn execute_scene_toggle_cycle(&mut self, room_name: &str, ts: Instant) -> Vec<Effect> {
        if let Some(effects) = self.execute_switch_steps(room_name, SwitchAction::ToggleCycle, ts) {
            return effects;
        }
        let scenes_for_now = self.scenes_for_room(room_name);
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();
        let off_transition = room.off_transition_seconds;

        if scenes_for_now.is_empty() {
            return Vec::new();
        }
        let cycle_window = Duration::from_secs_f64(self.defaults.cycle_window_seconds);

        let zone = self.world.light_zone(room_name);
        let is_on = zone.is_on();
        let prev_idx = zone.cycle_idx();
        let elapsed_since_last = zone.last_press_at.map(|last| ts.duration_since(last));
        let within_window = elapsed_since_last.is_some_and(|d| d < cycle_window);

        if !is_on {
            let first = scenes_for_now[0];
            tracing::info!(
                room = room_name,
                group = %group_name,
                scene = first,
                cycle_idx_to = 0,
                branch = "fresh on (was physically off)",
                "scene_toggle_cycle → scene_recall"
            );
            let effect = Effect::PublishGroupSet {
                room: room_idx,
                payload: Payload::scene_recall(first),
            };
            let owner = self.resolve_zone_owner(room_name, Owner::User);
            self.write_after_on(room_name, ts, 0, first, owner);
            self.propagate_to_descendants(room_name, true, ts);
            vec![effect]
        } else if within_window {
            let n = scenes_for_now.len();
            let next_idx = if zone.switch_cycle.is_some() {
                0
            } else {
                (prev_idx + 1) % n
            };
            let next_scene = scenes_for_now[next_idx];
            let elapsed_ms = elapsed_since_last
                .map(|d| d.as_millis())
                .unwrap_or(0);
            tracing::info!(
                room = room_name,
                group = %group_name,
                scene = next_scene,
                cycle_idx_from = prev_idx,
                cycle_idx_to = next_idx,
                cycle_len = n,
                elapsed_ms,
                branch = "cycle advance (within window)",
                "scene_toggle_cycle → scene_recall"
            );
            let effect = Effect::PublishGroupSet {
                room: room_idx,
                payload: Payload::scene_recall(next_scene),
            };
            let owner = self.resolve_zone_owner(room_name, Owner::User);
            self.write_after_on(room_name, ts, next_idx, next_scene, owner);
            self.propagate_to_descendants(room_name, true, ts);
            vec![effect]
        } else {
            let elapsed_ms = elapsed_since_last
                .map(|d| d.as_millis() as i64)
                .unwrap_or(-1);
            tracing::info!(
                room = room_name,
                group = %group_name,
                transition = off_transition,
                elapsed_ms,
                branch = "expire (cycle window passed)",
                "scene_toggle_cycle → state OFF"
            );
            let mut out = Vec::new();
            let owner = self.resolve_zone_owner(room_name, Owner::User);
            self.publish_off(room_name, room_idx, off_transition, ts, &mut out, owner);
            out
        }
    }

    /// `TurnOffRoom` effect -- turn off a room group with its configured
    /// off transition.
    pub(super) fn execute_turn_off_room(&mut self, room_name: &str, ts: Instant) -> Vec<Effect> {
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();
        let off_transition = room.off_transition_seconds;
        tracing::info!(
            room = room_name,
            group = %group_name,
            transition = off_transition,
            "turn_off_room → state OFF"
        );
        let mut out = Vec::new();
        let owner = self.resolve_zone_owner(room_name, Owner::User);
        self.publish_off(room_name, room_idx, off_transition, ts, &mut out, owner);
        out
    }

    /// `BrightnessStep` effect -- step brightness up or down.
    pub(super) fn execute_brightness_step(
        &mut self,
        room_name: &str,
        step: i16,
        transition: f64,
        ts: Instant,
    ) -> Vec<Effect> {
        if let Some(effects) = self.execute_switch_brightness(
            room_name, Payload::brightness_step(step, transition), ts,
        ) {
            return effects;
        }
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();
        tracing::info!(
            room = room_name,
            group = %group_name,
            step,
            transition,
            "brightness_step"
        );
        self.take_over_group_brightness(room_name, ts);
        vec![Effect::PublishGroupSet {
            room: room_idx,
            payload: Payload::brightness_step(step, transition),
        }]
    }

    /// `BrightnessMove` effect -- start continuous brightness change (hold).
    pub(super) fn execute_brightness_move(
        &mut self,
        room_name: &str,
        rate: i16,
        ts: Instant,
    ) -> Vec<Effect> {
        if let Some(effects) = self.execute_switch_brightness(
            room_name, Payload::brightness_move(rate), ts,
        ) {
            return effects;
        }
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();
        tracing::info!(
            room = room_name,
            group = %group_name,
            rate,
            "brightness_move"
        );
        self.take_over_group_brightness(room_name, ts);
        vec![Effect::PublishGroupSet {
            room: room_idx,
            payload: Payload::brightness_move(rate),
        }]
    }

    /// `BrightnessStop` effect -- stop continuous brightness change
    /// (hold release). Implemented as brightness_move with rate 0.
    pub(super) fn execute_brightness_stop(&mut self, room_name: &str, ts: Instant) -> Vec<Effect> {
        if let Some(effects) = self.execute_switch_brightness(
            room_name, Payload::brightness_move(0), ts,
        ) {
            return effects;
        }
        let Some(room_idx) = self.topology.room_idx(room_name) else {
            return Vec::new();
        };
        let room = self.topology.room(room_idx);
        let group_name = room.group_name.clone();
        tracing::info!(
            room = room_name,
            group = %group_name,
            "brightness_stop"
        );
        self.take_over_group_brightness(room_name, ts);
        vec![Effect::PublishGroupSet {
            room: room_idx,
            payload: Payload::brightness_move(0),
        }]
    }

    fn take_over_group_brightness(&mut self, room_name: &str, ts: Instant) {
        let lights = self
            .topology
            .room_by_name(room_name)
            .expect("known room")
            .light_members
            .clone();
        for endpoint in lights {
            let owner = if self.light_has_off_only_claim(endpoint.device) {
                Owner::Motion
            } else {
                Owner::User
            };
            let name = self.topology.device_name(endpoint.device).to_string();
            let light = self.world.light(&name);
            if light.is_on() {
                light.target.set_and_command(
                    LightTarget::On {
                        brightness: None,
                        color_temp: None,
                    },
                    owner,
                    ts,
                );
            } else {
                light.target.reassign_owner(owner, ts);
            }
        }
        let owner = self.resolve_zone_owner(room_name, Owner::User);
        self.world
            .light_zone(room_name)
            .target
            .reassign_owner(owner, ts);
    }

    pub(super) fn publish_off(
        &mut self,
        room_name: &str,
        room_idx: RoomIdx,
        off_transition: f64,
        ts: Instant,
        out: &mut Vec<Effect>,
        owner: Owner,
    ) {
        out.push(Effect::PublishGroupSet {
            room: room_idx,
            payload: Payload::state_off(off_transition),
        });
        self.write_after_off(room_name, ts, owner);
        self.propagate_to_descendants(room_name, false, ts);
    }

    /// TASS write-after-on: set target to On with the given owner,
    /// update last_press_at.
    fn write_after_on(
        &mut self,
        room_name: &str,
        ts: Instant,
        cycle_idx: usize,
        scene_id: u8,
        owner: Owner,
    ) {
        let scene = self
            .topology
            .room_by_name(room_name)
            .expect("known room")
            .scenes
            .scenes
            .iter()
            .find(|scene| scene.id == scene_id)
            .expect("validated scene");
        self.record_group_light_command(
            room_name,
            LightTarget::On {
                brightness: scene.brightness,
                color_temp: scene.color_temp,
            },
            owner,
            ts,
        );
        let zone = self.world.light_zone(room_name);
        zone.target.set_and_command(
            LightZoneTarget::On { scene_id, cycle_idx },
            owner,
            ts,
        );
        zone.last_press_at = Some(ts);
    }

    /// TASS write-after-off: set target to Off with the given owner,
    /// update timestamps.
    fn write_after_off(&mut self, room_name: &str, ts: Instant, owner: Owner) {
        self.record_group_light_command(room_name, LightTarget::Off, owner, ts);
        let zone = self.world.light_zone(room_name);
        zone.target.set_and_command(LightZoneTarget::Off, owner, ts);
        zone.last_press_at = Some(ts);
        zone.last_off_at = Some(ts);
    }

    /// Process z2m group state echo. Updates the light zone's actual
    /// state. On off-transitions, clears motion ownership and resets
    /// cycle state. OFF transitions also invalidate the affected member state.
    pub(super) fn handle_group_state(
        &mut self,
        group_name: &str,
        on: bool,
        ts: Instant,
    ) -> Vec<Effect> {
        let Some(room) = self.topology.room_by_group_name(group_name) else {
            return Vec::new();
        };
        let room_name = room.name.clone();

        let members = room.light_members.clone();
        let zone = self.world.light_zone(&room_name);
        let new_actual = if on { LightZoneActual::On } else { LightZoneActual::Off };
        // Use is_on() (target OR actual) not actual_is_on() — the target
        // may say On optimistically even before the first actual echo arrives.
        let was_on = zone.is_on();
        zone.actual.update(new_actual, ts);

        // A group OFF can arrive while the per-light step is still being
        // applied. It is an observation, not cancellation of that command.
        if zone.switch_cycle.is_some() {
            zone.target.mark_stale_if_old(ts, Self::TARGET_STALE_THRESHOLD);
            if !on && zone.target_is_on() && zone.target.is_actionable() {
                return Vec::new();
            }
        }

        // If actual now matches target and target is Commanded, advance
        // to Confirmed. Only for ON echoes — OFF transitions overwrite
        // the target below (making a confirm here redundant).
        if on && zone.switch_cycle.is_none() && matches!(zone.target.phase(), crate::tass::TargetPhase::Commanded | crate::tass::TargetPhase::Stale) {
            if let Some(LightZoneTarget::On { .. }) = zone.target.value() {
                zone.target.confirm(ts);
            }
        }

        if was_on == on {
            tracing::debug!(
                group = group_name,
                room = %room_name,
                state = on,
                "group state echo → no transition"
            );
            return Vec::new();
        }

        if !on {
            let descendants: Vec<_> = self
                .topology
                .rooms()
                .filter(|other| {
                    other.name != room_name
                        && !other.light_members.is_empty()
                        && other
                            .light_members
                            .iter()
                            .all(|light| members.contains(light))
                })
                .map(|other| other.name.clone())
                .collect();
            for name in descendants {
                let owner = self.resolve_zone_owner(&name, Owner::System);
                let zone = self.world.light_zone(&name);
                zone.actual.update(LightZoneActual::Off, ts);
                zone.switch_cycle = None;
                if zone.target_is_on() {
                    zone.target.set_and_command(LightZoneTarget::Off, owner, ts);
                    zone.target.confirm(ts);
                    zone.last_off_at = Some(ts);
                }
            }
            for endpoint in members {
                let name = self.topology.device_name(endpoint.device).to_string();
                let previous = self
                    .world
                    .lights
                    .get(&name)
                    .and_then(|light| light.actual.value())
                    .cloned();
                self.record_light_state(
                    &name,
                    false,
                    previous.as_ref().and_then(|actual| actual.brightness),
                    previous.as_ref().and_then(|actual| actual.color_temp),
                    previous.and_then(|actual| actual.color_xy),
                    ts,
                );
            }
        }

        if on {
            tracing::info!(
                group = group_name,
                room = %room_name,
                from = was_on,
                to = on,
                "group state echo → off→on transition (leaving user-owned)"
            );
        } else {
            // Special case for off-only rooms whose occupancy session
            // is still live: preserve `Owner::Motion` instead of wiping
            // to `Owner::System`. Without this, a user-driven off
            // inside the session loses the motion claim on the echo,
            // so the subsequent on would be user-owned and the
            // vacancy transition would no longer authorise the
            // auto-off.
            let preserve_motion = self.off_only_session_live(&room_name);
            let owner = if preserve_motion { Owner::Motion } else { Owner::System };
            let zone = self.world.light_zone(&room_name);
            zone.target.set_and_command(LightZoneTarget::Off, owner, ts);
            zone.target.confirm(ts);
            zone.switch_cycle = None;
            zone.last_off_at = Some(ts);
            tracing::info!(
                group = group_name,
                room = %room_name,
                from = was_on,
                to = on,
                ?owner,
                preserve_motion,
                "group state echo → on→off transition"
            );
        }

        Vec::new()
    }

    /// Optimistically propagate a parent's new physical state to every
    /// transitive descendant. Resets cycle state and motion ownership.
    pub(super) fn propagate_to_descendants(&mut self, ancestor: &str, on: bool, ts: Instant) {
        let Some(ancestor_idx) = self.topology.room_idx(ancestor) else {
            return;
        };
        let descendant_idxs: Vec<RoomIdx> =
            self.topology.descendants_of(ancestor_idx).to_vec();
        if descendant_idxs.is_empty() {
            return;
        }
        let descendants: Vec<RoomName> = descendant_idxs
            .iter()
            .map(|&idx| self.topology.room(idx).name.clone())
            .collect();
        tracing::info!(
            ancestor,
            descendants = ?descendants,
            physically_on = on,
            "propagating physical state to descendants (next press takes \
             toggle-off branch instead of fresh-on)"
        );
        for desc in descendants {
            // A descendant running off-only with a live session MUST
            // keep its Motion ownership even when an ancestor in a
            // different mode (OnOff/OnOnly) propagates. Without this
            // check, an ancestor's motion-on or button-press would
            // silently wipe the child's motion claim, and the child's
            // own vacancy would then fail the `is_motion_owned` gate
            // — leaving the child lit indefinitely.
            let preserve_motion = self.off_only_session_live(&desc);
            let owner = if preserve_motion { Owner::Motion } else { Owner::System };
            let zone = self.world.light_zone(&desc);
            zone.switch_cycle = None;
            if on {
                // Propagate on: align target/actual to match the
                // ancestor's physical state. `scene_id 0` is a
                // placeholder — the parent's scene_recall drove the
                // bulbs. `owner` preserves Motion when the descendant
                // is mid-session, else falls back to System so a
                // non-off-only child's stale motion claim (if any)
                // still gets cleared as before.
                zone.target.set_and_command(
                    LightZoneTarget::On { scene_id: 0, cycle_idx: 0 },
                    owner,
                    ts,
                );
                zone.actual.update(LightZoneActual::On, ts);
            } else {
                zone.target.set_and_command(LightZoneTarget::Off, owner, ts);
                zone.actual.update(LightZoneActual::Off, ts);
                zone.last_off_at = Some(ts);
            }
            zone.last_press_at = None;
        }
    }

    pub(super) fn handle_light_state(
        &mut self,
        device: &str,
        on: bool,
        brightness: Option<u8>,
        color_temp: Option<u16>,
        color_xy: Option<(f64, f64)>,
        ts: Instant,
    ) {
        self.record_light_state(device, on, brightness, color_temp, color_xy, ts);
        let Some(device_idx) = self.topology.device_idx(device) else {
            return;
        };
        let rooms: Vec<_> = self.topology.light_rooms(device_idx).collect();
        for idx in rooms {
            let room = self.topology.room(idx);
            let mut newest_on = None;
            let mut oldest_off = None;
            let mut all_known = true;
            for member in &room.light_members {
                let reading = self.world.lights
                    .get(self.topology.device_name(member.device))
                    .and_then(|light| light.actual.value().map(|value| (value, &light.actual)));
                match reading {
                    Some((value, actual)) => {
                        let since = actual.since().expect("known light state has a timestamp");
                        if value.on {
                            newest_on = Some(newest_on.map_or(since, |previous: Instant| previous.max(since)));
                        } else {
                            oldest_off = Some(oldest_off.map_or(since, |previous: Instant| previous.min(since)));
                        }
                    }
                    None => all_known = false,
                }
            }
            // One ON proves the zone is on; OFF requires every member. Keep
            // the age of the evidence, not the time of an unrelated report.
            let observed = newest_on.map(|since| (LightZoneActual::On, since))
                .or_else(|| oldest_off.filter(|_| all_known).map(|since| (LightZoneActual::Off, since)));
            let name = room.name.clone();
            let step_confirmed = self.switch_step_is_confirmed(&name);
            let step_is_off = self.switch_step_is_off(&name);
            let owner = self.resolve_zone_owner(&name, Owner::System);
            let zone = self.world.light_zone(&name);
            let Some((actual, since)) = observed else {
                zone.actual = TassActual::new();
                continue;
            };
            let was_on = zone.actual_is_on();
            zone.actual.update(actual, since);
            zone.target.mark_stale_if_old(ts, Self::TARGET_STALE_THRESHOLD);
            let follows_target = zone.target.since().is_some_and(|commanded| since >= commanded);
            if actual == LightZoneActual::Off {
                if was_on {
                    zone.last_off_at = Some(ts);
                }
                if zone.target_is_on() && !zone.target.is_actionable() && (follows_target || step_is_off) {
                    zone.target.adopt(LightZoneTarget::Off, owner, ts);
                    zone.switch_cycle = None;
                }
            }
            let matches = matches!((zone.target.value(), actual),
                (Some(LightZoneTarget::On { .. }), LightZoneActual::On)
                    | (Some(LightZoneTarget::Off), LightZoneActual::Off));
            let confirmation_is_current = if zone.switch_cycle.is_some() {
                step_confirmed
            } else {
                follows_target
            };
            if matches
                && confirmation_is_current
                && matches!(zone.target.phase(), TargetPhase::Commanded | TargetPhase::Stale)
            {
                zone.target.confirm(ts);
            }
        }
    }

    fn record_light_state(
        &mut self,
        device: &str,
        on: bool,
        brightness: Option<u8>,
        color_temp: Option<u16>,
        color_xy: Option<(f64, f64)>,
        ts: Instant,
    ) {
        let preserve_motion = self
            .topology
            .device_idx(device)
            .is_some_and(|idx| self.light_has_off_only_claim(idx));
        let light = self.world.light(device);
        light
            .target
            .mark_stale_if_old(ts, Self::TARGET_STALE_THRESHOLD);
        if !on
            && matches!(light.target.value(), Some(LightTarget::On { .. }))
            && !light.target.is_actionable()
        {
            light.target.adopt(
                LightTarget::Off,
                if preserve_motion {
                    Owner::Motion
                } else {
                    Owner::System
                },
                ts,
            );
        }
        let actual = crate::entities::light::LightActual {
            on,
            brightness,
            color_temp,
            color_xy,
        };
        if light
            .target
            .value()
            .is_some_and(|target| target.matches(&actual))
            && matches!(
                light.target.phase(),
                crate::tass::TargetPhase::Commanded | crate::tass::TargetPhase::Stale
            )
        {
            light.target.confirm(ts);
        }
        light.actual.update(actual, ts);
    }
}
