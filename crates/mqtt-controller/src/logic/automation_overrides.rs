//! Effective light and plug automation: deployed configuration with
//! dashboard overrides applied.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::config::scenes::{SceneSchedule, Slot, SlotName};
use crate::config::time_expr::TimeExpr;
use crate::settings::{KillSwitchOverride, ScheduleOwner};
use crate::topology::{ResolvedBinding, ResolvedMotionRule, ResolvedRoom, ResolvedTrigger};

use super::EventProcessor;

impl EventProcessor {
    pub(crate) fn room_slots<'a>(&'a self, room: &'a ResolvedRoom) -> &'a BTreeMap<SlotName, Slot> {
        self.settings.room_schedule_overrides.get(&room.name).unwrap_or(&room.scenes.slots)
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

    pub(crate) fn kill_switch_overridden(&self, binding: &str) -> bool {
        self.settings.kill_switch_overrides.contains_key(binding)
    }

    /// Effective time of an `At` binding; `None` for other triggers.
    pub(crate) fn timed_action_time<'a>(&'a self, binding: &'a ResolvedBinding) -> Option<&'a TimeExpr> {
        let ResolvedTrigger::At { time } = &binding.trigger else {
            return None;
        };
        Some(self.settings.timed_action_overrides.get(&binding.name).unwrap_or(time))
    }

    /// Effective threshold and holdoff of a `PowerBelow` binding; `None` for other triggers.
    pub(crate) fn kill_switch_params(&self, binding: &ResolvedBinding) -> Option<(f64, Duration)> {
        let ResolvedTrigger::PowerBelow { watts, holdoff, .. } = &binding.trigger else {
            return None;
        };
        Some(match self.settings.kill_switch_overrides.get(&binding.name) {
            Some(value) => (value.threshold_watts, Duration::from_secs(value.holdoff_secs)),
            None => (*watts, *holdoff),
        })
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

    /// Applies the same checks topology building applies to deployed schedules.
    pub(crate) fn validate_schedule_override(
        &self,
        owner: ScheduleOwner<'_>,
        slots: &BTreeMap<SlotName, Slot>,
    ) -> Result<(), String> {
        self.deployed_schedule(owner)?
            .validate_replacement_slots(slots)
            .map_err(|error| error.to_string())?;
        if self.location.is_none() && slots.values().any(Slot::uses_sun) {
            return Err("Sunrise/sunset times require a configured location".into());
        }
        if let ScheduleOwner::MotionRule(_) = owner {
            if let Some((name, _)) = slots.iter().find(|(_, slot)| slot.scene_ids.is_empty()) {
                return Err(format!("slot {name:?} has no scenes"));
            }
        }
        Ok(())
    }

    pub(crate) fn apply_schedule_override(
        &mut self,
        owner: ScheduleOwner<'_>,
        slots: Option<BTreeMap<SlotName, Slot>>,
    ) {
        let overrides = match owner {
            ScheduleOwner::Room(_) => &mut self.settings.room_schedule_overrides,
            ScheduleOwner::MotionRule(_) => &mut self.settings.motion_schedule_overrides,
        };
        match slots {
            Some(slots) => overrides.insert(owner.name().into(), slots),
            None => overrides.remove(owner.name()),
        };
    }

    pub(crate) fn validate_timed_action(&self, binding: &str) -> Result<(), String> {
        match self.topology.bindings().iter().find(|resolved| resolved.name == binding) {
            Some(ResolvedBinding { trigger: ResolvedTrigger::At { .. }, .. }) => Ok(()),
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

    pub(crate) fn validate_kill_switch(&self, binding: &str) -> Result<(), String> {
        match self.topology.bindings().iter().find(|resolved| resolved.name == binding) {
            Some(ResolvedBinding { trigger: ResolvedTrigger::PowerBelow { .. }, .. }) => Ok(()),
            Some(_) => Err(format!("Binding is not a kill switch: {binding}")),
            None => Err(format!("Unknown binding: {binding}")),
        }
    }

    /// A deployment can remove or reshape what a stored override refers to.
    /// Such overrides stay in the database but are not applied.
    pub(super) fn discard_inapplicable_overrides(&mut self) {
        let mut rooms = std::mem::take(&mut self.settings.room_schedule_overrides);
        rooms.retain(|name, slots| keep_override("room schedule", name,
            self.validate_schedule_override(ScheduleOwner::Room(name), slots)));
        self.settings.room_schedule_overrides = rooms;
        let mut rules = std::mem::take(&mut self.settings.motion_schedule_overrides);
        rules.retain(|name, slots| keep_override("motion schedule", name,
            self.validate_schedule_override(ScheduleOwner::MotionRule(name), slots)));
        self.settings.motion_schedule_overrides = rules;
        let mut times = std::mem::take(&mut self.settings.timed_action_overrides);
        times.retain(|name, time| keep_override("timed action", name,
            self.validate_timed_action(name).and_then(|()| self.validate_timed_action_time(time))));
        self.settings.timed_action_overrides = times;
        let mut kill_switches = std::mem::take(&mut self.settings.kill_switch_overrides);
        kill_switches.retain(|name, value: &mut KillSwitchOverride| keep_override("kill switch", name,
            self.validate_kill_switch(name).and_then(|()| value.validate())));
        self.settings.kill_switch_overrides = kill_switches;
    }
}

fn keep_override(kind: &str, name: &str, validation: Result<(), String>) -> bool {
    match validation {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(kind, name, %error, "stored override does not fit the deployed configuration; using deployed values");
            false
        }
    }
}
