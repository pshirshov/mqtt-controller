//! Effective light and plug automation: deployed configuration with
//! dashboard overrides applied.

use std::collections::BTreeMap;
use std::time::Duration;

use mqtt_controller_wire::PlugAction;

use crate::config::scenes::{SceneSchedule, Slot, SlotName};
use crate::config::time_expr::TimeExpr;
use crate::settings::{PlugScheduleOverride, RoomScheduleOverride, ScheduleOwner};
use crate::topology::{
    DeviceIdx, PlugIdx, ResolvedBinding, ResolvedEffect, ResolvedMotionRule, ResolvedRoom,
    ResolvedSwitchStep, ResolvedTrigger,
};

use super::EventProcessor;

/// Switch steps of one room's override, resolved against the topology.
pub(crate) type ResolvedSwitchSteps = BTreeMap<SlotName, Vec<ResolvedSwitchStep>>;

impl EventProcessor {
    pub(crate) fn room_slots<'a>(&'a self, room: &'a ResolvedRoom) -> &'a BTreeMap<SlotName, Slot> {
        self.settings.room_schedule_overrides.get(&room.name).map_or(&room.scenes.slots, |value| &value.slots)
    }

    pub(crate) fn room_switch_steps<'a>(&'a self, room: &'a ResolvedRoom) -> &'a ResolvedSwitchSteps {
        self.resolved_switch_steps.get(&room.name).unwrap_or(&room.switch_steps)
    }

    pub(crate) fn motion_slots<'a>(&'a self, rule: &'a ResolvedMotionRule) -> &'a BTreeMap<SlotName, Slot> {
        self.settings.motion_schedule_overrides.get(&rule.name).unwrap_or(&rule.scenes.slots)
    }

    pub(crate) fn schedule_overridden(&self, owner: ScheduleOwner<'_>) -> bool {
        match owner {
            ScheduleOwner::Room(name) => self.settings.room_schedule_overrides.contains_key(name),
            ScheduleOwner::MotionRule(name) => self.settings.motion_schedule_overrides.contains_key(name),
        }
    }

    pub(crate) fn timed_action_overridden(&self, binding: &str) -> bool {
        self.settings.timed_action_overrides.contains_key(binding)
    }

    pub(crate) fn plug_schedule_overridden(&self, device: &str) -> bool {
        self.settings.plug_schedule_overrides.contains_key(device)
    }

    /// Effective time of an `At` binding; `None` for other triggers.
    pub(crate) fn timed_action_time<'a>(&'a self, binding: &'a ResolvedBinding) -> Option<&'a TimeExpr> {
        let ResolvedTrigger::At { time } = &binding.trigger else {
            return None;
        };
        Some(self.settings.timed_action_overrides.get(&binding.name).unwrap_or(time))
    }

    /// Threshold and holdoff of a `PowerBelow` binding; `None` for other triggers.
    pub(crate) fn kill_switch_params(&self, binding: &ResolvedBinding) -> Option<(f64, Duration)> {
        let ResolvedTrigger::PowerBelow { watts, holdoff, .. } = &binding.trigger else {
            return None;
        };
        Some((*watts, *holdoff))
    }

    /// Whether a plug schedule override supersedes this deployed binding.
    fn binding_superseded(&self, binding: &ResolvedBinding) -> bool {
        let plug = match &binding.trigger {
            ResolvedTrigger::PowerBelow { plug, .. } => Some(*plug),
            ResolvedTrigger::At { .. } => binding.effect.target_plug(),
            ResolvedTrigger::Button { .. } => None,
        };
        plug.is_some_and(|plug| self.plug_schedule_overridden(self.topology.device_name(plug.device())))
    }

    /// Deployed bindings not superseded by plug overrides, plus the bindings
    /// those overrides define.
    pub(crate) fn effective_bindings(&self) -> Vec<ResolvedBinding> {
        self.topology
            .bindings()
            .iter()
            .filter(|binding| !self.binding_superseded(binding))
            .cloned()
            .chain(self.resolved_plug_bindings.values().flatten().cloned())
            .collect()
    }

    /// Effective `PowerBelow` bindings watching a plug.
    pub(crate) fn power_below_bindings(&self, device: DeviceIdx) -> Vec<ResolvedBinding> {
        match self.resolved_plug_bindings.get(self.topology.device_name(device)) {
            Some(bindings) => bindings
                .iter()
                .filter(|binding| matches!(binding.trigger, ResolvedTrigger::PowerBelow { .. }))
                .cloned()
                .collect(),
            None => self
                .topology
                .bindings_for_power_below(device)
                .iter()
                .map(|&idx| self.topology.binding(idx).clone())
                .collect(),
        }
    }

    fn deployed_schedule(&self, owner: ScheduleOwner<'_>) -> Result<&SceneSchedule, String> {
        match owner {
            ScheduleOwner::Room(name) => self.topology.room_by_name(name)
                .map(|room| &room.scenes)
                .ok_or_else(|| format!("Unknown light group: {name}")),
            ScheduleOwner::MotionRule(name) => self.topology.motion_rules().iter()
                .find(|rule| rule.name == name)
                .map(|rule| &rule.scenes)
                .ok_or_else(|| format!("Unknown motion rule: {name}")),
        }
    }

    pub(crate) fn validate_schedule_owner(&self, owner: ScheduleOwner<'_>) -> Result<(), String> {
        self.deployed_schedule(owner).map(|_| ())
    }

    fn validate_slots(&self, deployed: &SceneSchedule, slots: &BTreeMap<SlotName, Slot>) -> Result<(), String> {
        if slots.is_empty() {
            return Err("The schedule needs at least one slot".into());
        }
        if let Some(name) = slots.keys().find(|name| name.trim().is_empty()) {
            return Err(format!("Slot name {name:?} must not be blank"));
        }
        SceneSchedule { scenes: deployed.scenes.clone(), slots: slots.clone() }
            .validate()
            .map_err(|error| error.to_string())?;
        if self.location.is_none() && slots.values().any(Slot::uses_sun) {
            return Err("Sunrise/sunset times require a configured location".into());
        }
        Ok(())
    }

    /// Applies the checks topology building applies to deployed schedules
    /// and switch steps; the slot set may differ from the deployed one.
    pub(crate) fn validate_room_override(
        &self,
        room: &str,
        value: &RoomScheduleOverride,
    ) -> Result<ResolvedSwitchSteps, String> {
        let resolved = self.topology.room_by_name(room).ok_or_else(|| format!("Unknown light group: {room}"))?;
        self.validate_slots(&resolved.scenes, &value.slots)?;
        let schedule = SceneSchedule { scenes: resolved.scenes.scenes.clone(), slots: value.slots.clone() };
        let steps = crate::topology::resolve_switch_steps(
            room, &schedule, &value.switch_steps, &resolved.members, &resolved.light_members,
        ).map_err(|error| error.to_string())?;
        if !steps.is_empty() {
            crate::topology::switch_step_endpoints_observable(resolved, self.topology.rooms())
                .map_err(|error| error.to_string())?;
        }
        Ok(steps)
    }

    /// Motion targets refer to the deployed slot names, so only the slot
    /// boundaries and scenes may change.
    pub(crate) fn validate_motion_override(
        &self,
        rule: &str,
        slots: &BTreeMap<SlotName, Slot>,
    ) -> Result<(), String> {
        let deployed = self.deployed_schedule(ScheduleOwner::MotionRule(rule))?;
        deployed.validate_replacement_slots(slots).map_err(|error| error.to_string())?;
        if self.location.is_none() && slots.values().any(Slot::uses_sun) {
            return Err("Sunrise/sunset times require a configured location".into());
        }
        if let Some((name, _)) = slots.iter().find(|(_, slot)| slot.scene_ids.is_empty()) {
            return Err(format!("slot {name:?} has no scenes"));
        }
        Ok(())
    }

    /// Replaces or removes a room override. The active switch-step
    /// selection refers to the previous steps, so it is forgotten.
    pub(crate) fn apply_room_override(
        &mut self,
        room: &str,
        value: Option<(RoomScheduleOverride, ResolvedSwitchSteps)>,
    ) {
        match value {
            Some((value, steps)) => {
                self.settings.room_schedule_overrides.insert(room.into(), value);
                self.resolved_switch_steps.insert(room.into(), steps);
            }
            None => {
                self.settings.room_schedule_overrides.remove(room);
                self.resolved_switch_steps.remove(room);
            }
        }
        if let Some(zone) = self.world.light_zones.get_mut(room) {
            zone.switch_cycle = None;
        }
    }

    pub(crate) fn apply_motion_override(&mut self, rule: &str, slots: Option<BTreeMap<SlotName, Slot>>) {
        match slots {
            Some(slots) => self.settings.motion_schedule_overrides.insert(rule.into(), slots),
            None => self.settings.motion_schedule_overrides.remove(rule),
        };
    }

    pub(crate) fn validate_plug(&self, device: &str) -> Result<PlugIdx, String> {
        self.topology.plug_idx_by_name(device).ok_or_else(|| format!("Unknown plug: {device}"))
    }

    /// Applies the checks topology building applies to deployed `at` and
    /// `power_below` bindings, then resolves the override into bindings.
    pub(crate) fn validate_plug_override(
        &self,
        device: &str,
        value: &PlugScheduleOverride,
    ) -> Result<Vec<ResolvedBinding>, String> {
        let plug = self.validate_plug(device)?;
        let mut bindings = Vec::new();
        for (index, action) in value.timed_actions.iter().enumerate() {
            self.validate_timed_action_time(&action.time)?;
            let effect = match action.action {
                PlugAction::On => ResolvedEffect::TurnOn { plug },
                PlugAction::Off => ResolvedEffect::TurnOff { plug },
                PlugAction::Toggle => ResolvedEffect::Toggle { plug, confirm_off_seconds: None },
            };
            bindings.push(ResolvedBinding {
                name: format!("plug-schedule:{device}:{}", index + 1),
                trigger: ResolvedTrigger::At { time: action.time.clone() },
                effect,
            });
        }
        if let Some(kill_switch) = &value.kill_switch {
            kill_switch.validate()?;
            if !self.topology.power_metered(plug.device()) {
                return Err(format!("{device} does not report power, so it cannot have a kill switch"));
            }
            bindings.push(ResolvedBinding {
                name: format!("plug-schedule:{device}:kill-switch"),
                trigger: ResolvedTrigger::PowerBelow {
                    plug,
                    watts: kill_switch.threshold_watts,
                    holdoff: Duration::from_secs(kill_switch.holdoff_secs),
                },
                effect: ResolvedEffect::TurnOff { plug },
            });
        }
        Ok(bindings)
    }

    /// Replaces or removes a plug override. Kill-switch state belongs to the
    /// previous rules, so it restarts from the plug's current reading.
    pub(crate) fn apply_plug_override(
        &mut self,
        device: &str,
        value: Option<(PlugScheduleOverride, Vec<ResolvedBinding>)>,
    ) {
        let now = self.clock.now();
        match value {
            Some((value, bindings)) => {
                self.settings.plug_schedule_overrides.insert(device.into(), value);
                self.resolved_plug_bindings.insert(device.into(), bindings);
            }
            None => {
                self.settings.plug_schedule_overrides.remove(device);
                self.resolved_plug_bindings.remove(device);
            }
        }
        let Some(plug) = self.world.plugs.get_mut(device) else {
            return;
        };
        plug.kill_switch_rules.clear();
        if plug.is_on() {
            let power = plug.power();
            self.arm_kill_switch_rules(device, power, now, super::plugs::ArmCause::OffOnTransition);
        }
    }

    /// Only room-targeting timed bindings take per-binding time overrides;
    /// plug timings are edited as a whole plug schedule.
    pub(crate) fn validate_timed_action(&self, binding: &str) -> Result<(), String> {
        match self.topology.bindings().iter().find(|resolved| resolved.name == binding) {
            Some(ResolvedBinding { trigger: ResolvedTrigger::At { .. }, effect, .. }) if effect.target_plug().is_none() => Ok(()),
            Some(ResolvedBinding { trigger: ResolvedTrigger::At { .. }, .. }) => Err(format!("Binding times a plug; edit the plug schedule instead: {binding}")),
            Some(_) => Err(format!("Binding is not a timed action: {binding}")),
            None => Err(format!("Unknown binding: {binding}")),
        }
    }

    pub(crate) fn validate_timed_action_time(&self, time: &TimeExpr) -> Result<(), String> {
        if !time.is_valid_trigger_time() {
            return Err(format!("Timed actions cannot run at {time}"));
        }
        if time.uses_sun() && self.location.is_none() {
            return Err("Sunrise/sunset times require a configured location".into());
        }
        Ok(())
    }

    /// A deployment can remove or reshape what a stored override refers to.
    /// Such overrides stay in the database but are not applied.
    pub(super) fn discard_inapplicable_overrides(&mut self) {
        let rooms = std::mem::take(&mut self.settings.room_schedule_overrides);
        self.resolved_switch_steps.clear();
        for (name, value) in rooms {
            match self.validate_room_override(&name, &value) {
                Ok(steps) => self.apply_room_override(&name, Some((value, steps))),
                Err(error) => log_discarded("room schedule", &name, &error),
            }
        }
        let mut rules = std::mem::take(&mut self.settings.motion_schedule_overrides);
        rules.retain(|name, slots| keep_override("motion schedule", name,
            self.validate_motion_override(name, slots)));
        self.settings.motion_schedule_overrides = rules;
        let mut times = std::mem::take(&mut self.settings.timed_action_overrides);
        times.retain(|name, time| keep_override("timed action", name,
            self.validate_timed_action(name).and_then(|()| self.validate_timed_action_time(time))));
        self.settings.timed_action_overrides = times;
        let plugs = std::mem::take(&mut self.settings.plug_schedule_overrides);
        self.resolved_plug_bindings.clear();
        for (device, value) in plugs {
            match self.validate_plug_override(&device, &value) {
                Ok(bindings) => self.apply_plug_override(&device, Some((value, bindings))),
                Err(error) => log_discarded("plug schedule", &device, &error),
            }
        }
    }
}

fn keep_override(kind: &str, name: &str, validation: Result<(), String>) -> bool {
    match validation {
        Ok(()) => true,
        Err(error) => {
            log_discarded(kind, name, &error);
            false
        }
    }
}

fn log_discarded(kind: &str, name: &str, error: &str) {
    tracing::warn!(kind, name, error, "stored override does not fit the deployed configuration; using deployed values");
}
