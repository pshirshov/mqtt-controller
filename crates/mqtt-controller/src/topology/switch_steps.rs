use std::collections::{BTreeMap, BTreeSet};

use crate::config::room::SwitchStep;
use crate::config::{Room, SceneSchedule};

use super::{LightEndpoint, ResolvedRoom, ResolvedSwitchStep, TopologyError};

pub(super) fn validate_endpoints(rooms: &[ResolvedRoom]) -> Result<(), TopologyError> {
    for room in rooms.iter().filter(|room| !room.switch_steps.is_empty()) {
        endpoints_observable(room, rooms.iter())?;
    }
    Ok(())
}

/// Switch steps observe members per device, so a stepped room's lights
/// must not appear under another endpoint anywhere in the topology.
pub(crate) fn endpoints_observable<'a>(
    room: &ResolvedRoom,
    rooms: impl Iterator<Item = &'a ResolvedRoom>,
) -> Result<(), TopologyError> {
    let other_endpoints: Vec<LightEndpoint> = rooms.flat_map(|room| room.light_members.iter().copied()).collect();
    for member in &room.light_members {
        if other_endpoints
            .iter()
            .any(|other| other.device == member.device && other.endpoint != member.endpoint)
        {
            return Err(TopologyError::InvalidSwitchSteps {
                room: room.name.clone(),
                reason: "requires one configured endpoint per light; endpoint-specific telemetry is unavailable".into(),
            });
        }
    }
    Ok(())
}

pub(super) fn resolve_room(
    room: &Room,
    members: &[LightEndpoint],
) -> Result<BTreeMap<String, Vec<ResolvedSwitchStep>>, TopologyError> {
    resolve(&room.name, &room.scenes, &room.switch_steps, &room.members, members)
}

/// Resolve per-slot switch steps against `scenes` (whose slots may be a
/// dashboard override) and the room's members.
pub(crate) fn resolve(
    room_name: &str,
    scenes: &SceneSchedule,
    switch_steps: &BTreeMap<String, Vec<SwitchStep>>,
    member_keys: &[String],
    members: &[LightEndpoint],
) -> Result<BTreeMap<String, Vec<ResolvedSwitchStep>>, TopologyError> {
    let invalid = |reason: String| TopologyError::InvalidSwitchSteps {
        room: room_name.to_string(),
        reason,
    };
    if !switch_steps.is_empty()
        && members
            .iter()
            .map(|member| member.device)
            .collect::<BTreeSet<_>>()
            .len()
            != members.len()
    {
        return Err(invalid("requires one endpoint per light; duplicate device members cannot be observed independently".into()));
    }
    switch_steps
        .iter()
        .map(|(slot, steps)| {
            if !scenes.slots.contains_key(slot) {
                return Err(invalid(format!("unknown slot {slot:?}")));
            }
            if steps.is_empty() {
                return Err(invalid(format!("slot {slot:?} has an empty step sequence")));
            }
            let steps = steps
                .iter()
                .map(|step| {
                    let scene = scenes
                        .scenes
                        .iter()
                        .find(|scene| scene.id == step.scene_id)
                        .ok_or_else(|| invalid(format!("unknown scene {}", step.scene_id)))?;
                    if scene.state != "ON"
                        || !scene.transition.is_finite()
                        || scene.transition < 0.0
                    {
                        return Err(invalid(format!(
                            "scene {} must turn lights on with a finite nonnegative transition",
                            scene.id
                        )));
                    }
                    if step.lights.is_empty() {
                        return Err(invalid("step has an empty light selection".into()));
                    }
                    if step.lights.iter().collect::<BTreeSet<_>>().len() != step.lights.len() {
                        return Err(invalid("step has duplicate lights".into()));
                    }
                    let lights = step
                        .lights
                        .iter()
                        .map(|light| {
                            let index = member_keys
                                .iter()
                                .position(|member| member == light)
                                .ok_or_else(|| {
                                    invalid(format!("{light:?} is not a room member"))
                                })?;
                            Ok(members[index])
                        })
                        .collect::<Result<_, TopologyError>>()?;
                    Ok(ResolvedSwitchStep {
                        scene: scene.clone(),
                        lights,
                    })
                })
                .collect::<Result<_, TopologyError>>()?;
            Ok((slot.clone(), steps))
        })
        .collect()
}
