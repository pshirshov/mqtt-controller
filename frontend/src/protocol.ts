import { z } from 'zod';

const number = z.number().finite();
const timestamp = number.int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const maybeNumber = number.nullish();
const actualMeta = z.object({ freshness: z.string().default('unknown'), since_ago_ms: timestamp.nullish() });
const targetMeta = z.object({ phase: z.string().default('unset'), owner: z.string().default(''), since_ago_ms: timestamp.nullish() });
const tass = { target: targetMeta.nullish(), actual: actualMeta.nullish() };
const switchInfo = z.object({
  device: z.string(), buttons: z.array(z.object({ button: z.string(), actions: z.array(z.object({ gesture: z.string(), description: z.string() })) })),
});
// The Rust serde names are snake_case on target values, PascalCase on messages.
const lightTargetWire = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('off') }),
  z.object({ kind: z.literal('on'), brightness: maybeNumber, color_temp: maybeNumber }),
]);
export const lightSchema = z.object({
  ...tass, device: z.string(), room: z.string().nullish(), target_value: lightTargetWire.nullish(),
  actual_value: z.object({ on: z.boolean(), brightness: maybeNumber, color_temp: maybeNumber, color_xy: z.tuple([number, number]).nullish() }).nullish(),
});
const sceneId = number.int().min(0).max(255);
export const slotPlanSchema = z.object({ name: z.string(), from: z.string(), to: z.string(), scene_ids: z.array(sceneId) });
const sceneOptionSchema = z.object({ id: sceneId, name: z.string() });
const schedulePlanSchema = z.object({ slots: z.array(slotPlanSchema), available_scenes: z.array(sceneOptionSchema), overridden: z.boolean() });
const noSchedule = { slots: [], available_scenes: [], overridden: false };
export const switchStepSchema = z.object({ scene_id: sceneId, lights: z.array(z.string()) });
const switchStepsSchema = z.record(z.string(), z.array(switchStepSchema));
const timedActionSchema = z.object({ binding: z.string(), time: z.string(), action: z.string(), overridden: z.boolean() });
const plugActionSchema = z.enum(['on', 'off', 'toggle']);
const plugTimedActionSchema = z.object({ time: z.string(), action: plugActionSchema });
const killSwitchPlanSchema = z.object({ threshold_watts: number, holdoff_secs: number.int() });
const plugScheduleSchema = z.object({
  timed_actions: z.array(plugTimedActionSchema), kill_switch: killSwitchPlanSchema.nullish(), overridden: z.boolean(), power_metered: z.boolean(),
});
const noPlugSchedule = { timed_actions: [], kill_switch: null, overridden: false, power_metered: false };
const motion = z.object({
  name: z.string(), mode: z.enum(['on-off', 'on-only', 'off-only']), active_slot: z.string().nullable(),
  targets: z.array(z.string()), session_targets: z.array(z.string()), max_illuminance: maybeNumber,
  off_cooldown_secs: number, cooldown_remaining_secs: maybeNumber,
  sensors: z.array(z.object({ device: z.string(), occupied: z.boolean().nullish(), illuminance: maybeNumber,
    last_event: z.object({ timestamp_epoch_ms: timestamp, kind: z.enum(['motion', 'clear']) }).nullable(),
    freshness: z.string().default('unknown'), since_ago_ms: timestamp.nullish(), occupancy_timeout_secs: number.default(0) })),
  schedule: schedulePlanSchema.default(noSchedule),
});
export const roomSchema = z.object({
  ...tass, name: z.string(), group_name: z.string(), room: z.string(), physically_on: z.boolean(), motion_owned: z.boolean(), motion_enabled: z.boolean(),
  active_slot: z.string().nullable(), scene_ids: z.array(number.int()), cycle_idx: number.int(),
  target_value: z.discriminatedUnion('kind', [z.object({ kind: z.literal('off') }), z.object({ kind: z.literal('on'), scene_id: number.int(), cycle_idx: number.int() })]).nullish(),
  actual_value: z.enum(['on', 'off']).nullish(), switches: z.array(switchInfo).default([]),
  lights: z.array(z.object({ device: z.string() })).default([]), motion_rules: z.array(motion).default([]),
  schedule: schedulePlanSchema.default(noSchedule), members: z.array(z.string()).default([]), switch_steps: switchStepsSchema.default({}),
  timed_actions: z.array(timedActionSchema).default([]),
});
export const plugSchema = z.object({
  exclude_from_totals: z.boolean().default(false),
  ...tass, device: z.string(), display_name: z.string().nullish(), room: z.string().nullish(), on: z.boolean(), target_value: z.enum(['on', 'off']).nullish(),
  actual_value: z.object({ on: z.boolean(), power: maybeNumber }).nullish(), power_watts: maybeNumber,
  power_actual: actualMeta.nullable(),
  idle_since_ago_ms: timestamp.nullable(), kill_switch_holdoff_secs: maybeNumber,
  kill_switch_rules: z.array(z.object({ rule_name: z.string(), state: z.string(), threshold_watts: number, holdoff_secs: number, idle_since_ago_ms: timestamp.nullish(),
    overridden: z.boolean().default(false) })).default([]),
  linked_switches: z.array(switchInfo).default([]), schedule: plugScheduleSchema.default(noPlugSchedule),
});
export const valveTargetSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('setpoint'), temperature: number }), z.object({ kind: z.literal('inhibited') }),
  z.object({ kind: z.literal('forced_open'), reason: z.string() }),
]);
const trvRunningStateSchema = z.enum(['unknown', 'idle', 'heat']);
const scheduleRangeSchema = z.object({ start: z.string(), end: z.string(), temperature: number });
export const valveScheduleSchema = z.object({ days: z.record(
  z.enum(['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday']),
  z.array(scheduleRangeSchema),
) });
export const valveSchema = z.object({
  heat_demand_enabled: z.boolean().default(true),
  boost: z.object({ temperature: number, ends_at_epoch_ms: timestamp, remaining_ms: timestamp }).nullable().default(null),
  ...tass, device: z.string(), local_temperature: number.nullable(), setpoint: number.nullable(),
  pi_heating_demand: number.nullable(), battery: number.nullable(), running_state: trvRunningStateSchema,
  inhibited: z.boolean(), forced: z.boolean().default(false), schedule: z.string().default(''),
  schedule_summary: z.string().default(''), target_value: valveTargetSchema.nullish(),
  schedule_override: z.boolean().default(false), schedule_plan: valveScheduleSchema.nullable().default(null),
});
export const heatingSchema = z.object({
  ...tass, name: z.string(), relay_device: z.string(), relay_on: z.boolean(), relay_state_known: z.boolean(),
  relay_temperature: number.nullable(), relay_stale: z.boolean().default(false),
  min_cycle_remaining_secs: number.default(0), min_pause_remaining_secs: number.default(0),
  target_value: z.enum(['heating', 'off']).nullish(),
  actual_value: z.object({ relay_on: z.boolean(), temperature: maybeNumber }).nullish(), trvs: z.array(valveSchema),
});
export const historyPointSchema = z.object({
  timestamp_epoch_ms: timestamp, observed_at_epoch_ms: timestamp.nullable(), local_temperature: number.nullable(),
  reported_setpoint: number.nullable(), target: valveTargetSchema.nullable(), heating_demand: number.nullable(),
  running_state: trvRunningStateSchema.default('unknown'),
  battery: number.nullable(), freshness: z.string(),
});
export const commandSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('RecallScene'), room: z.string(), scene_id: number.int().min(0).max(255) }),
  z.object({ kind: z.literal('SetRoomOff'), room: z.string() }),
  z.object({ kind: z.literal('SetMotionEnabled'), room: z.string(), enabled: z.boolean() }),
  z.object({ kind: z.literal('SetHeatDemandEnabled'), device: z.string(), enabled: z.boolean() }),
  z.object({ kind: z.literal('StartValveBoost'), device: z.string(), duration_minutes: z.number().int(), temperature: number }),
  z.object({ kind: z.literal('SetValveBoostTarget'), device: z.string(), temperature: number }),
  z.object({ kind: z.literal('CancelValveBoost'), device: z.string() }),
  z.object({ kind: z.literal('SetValveSchedule'), device: z.string(), schedule: valveScheduleSchema }),
  z.object({ kind: z.literal('ResetValveSchedule'), device: z.string() }),
  z.object({ kind: z.literal('SetRoomSchedule'), room: z.string(), slots: z.array(slotPlanSchema), switch_steps: switchStepsSchema }),
  z.object({ kind: z.literal('ResetRoomSchedule'), room: z.string() }),
  z.object({ kind: z.literal('SetMotionSchedule'), rule: z.string(), slots: z.array(slotPlanSchema) }),
  z.object({ kind: z.literal('ResetMotionSchedule'), rule: z.string() }),
  z.object({ kind: z.literal('SetTimedActionTime'), binding: z.string(), time: z.string() }),
  z.object({ kind: z.literal('ResetTimedActionTime'), binding: z.string() }),
  z.object({ kind: z.literal('SetPlugSchedule'), device: z.string(), timed_actions: z.array(plugTimedActionSchema), kill_switch: killSwitchPlanSchema.nullable() }),
  z.object({ kind: z.literal('ResetPlugSchedule'), device: z.string() }),
  z.object({ kind: z.literal('SetPlugPower'), device: z.string(), on: z.boolean() }),
]);
export const snapshotSchema = z.object({
  type: z.literal('StateSnapshot'), rooms: z.array(roomSchema), plugs: z.array(plugSchema),
  heating_zones: z.array(heatingSchema).default([]), lights: z.array(lightSchema).default([]), timestamp_epoch_ms: timestamp,
});
const entitySchema = z.discriminatedUnion('kind', [
  z.object({ type: z.literal('Entity'), kind: z.literal('Room'), data: roomSchema }),
  z.object({ type: z.literal('Entity'), kind: z.literal('Plug'), data: plugSchema }),
  z.object({ type: z.literal('Entity'), kind: z.literal('HeatingZone'), data: heatingSchema }),
  z.object({ type: z.literal('Entity'), kind: z.literal('Light'), data: lightSchema }),
]);
export const historySchema = z.object({
  type: z.literal('ValveHistory'), request_id: z.string(), device: z.string(), from_epoch_ms: timestamp, to_epoch_ms: timestamp,
  points: z.array(historyPointSchema), error: z.string().nullable(),
});
export const plugPowerHistorySchema = z.object({
  type: z.literal('PlugPowerHistory'), request_id: z.string(), device: z.string(), from_epoch_ms: timestamp, to_epoch_ms: timestamp,
  points: z.array(z.object({ timestamp_epoch_ms: timestamp, power_watts: number.nullable(), freshness: z.string() })),
  estimated_energy_kwh: number.nonnegative().nullable(), energy_observed_ms: timestamp, error: z.string().nullable(),
});
export const heatingEnergyHistorySchema = z.object({
  type: z.literal('HeatingEnergyHistory'), request_id: z.string(), from_epoch_ms: timestamp, to_epoch_ms: timestamp,
  points: z.array(z.object({
    timestamp_epoch_ms: timestamp,
    relays: z.array(z.object({ zone: z.string(), device: z.string(), on: z.boolean().nullable(), freshness: z.string() })),
    heat_pump: z.object({ device: z.string(), power_watts: number.nonnegative().nullable(), energy_kwh: number.nonnegative().nullable(),
      power_freshness: z.string(), energy_freshness: z.string() }).nullable(),
  })),
  error: z.string().nullable(),
});
export const serverSchema = z.union([
  snapshotSchema, entitySchema, historySchema, plugPowerHistorySchema, heatingEnergyHistorySchema,
  z.object({ type: z.literal('CommandResult'), request_id: z.string(), error: z.string().nullable() }),
  z.object({ type: z.literal('Pong'), nonce: z.string(), client_ts_ms: timestamp, server_ts_ms: timestamp }),
  z.object({ type: z.literal('EventLog') }), z.object({ type: z.literal('EntityLog') }), z.object({ type: z.literal('Topology') }),
]);
export type Room = z.infer<typeof roomSchema>;
export type Plug = z.infer<typeof plugSchema>;
export type Light = z.infer<typeof lightSchema>;
export type Valve = z.infer<typeof valveSchema>;
export type ValveSchedule = z.infer<typeof valveScheduleSchema>;
export type SlotPlan = z.infer<typeof slotPlanSchema>;
export type SceneOption = z.infer<typeof sceneOptionSchema>;
export type SwitchStep = z.infer<typeof switchStepSchema>;
export type SwitchSteps = z.infer<typeof switchStepsSchema>;
export type SchedulePlan = z.infer<typeof schedulePlanSchema>;
export type TimedAction = z.infer<typeof timedActionSchema>;
export type KillSwitchRule = Plug['kill_switch_rules'][number];
export type PlugAction = z.infer<typeof plugActionSchema>;
export type PlugSchedule = z.infer<typeof plugScheduleSchema>;
export type HeatingZone = z.infer<typeof heatingSchema>;
export type ActualMeta = z.infer<typeof actualMeta>;
export type TargetMeta = z.infer<typeof targetMeta>;
export type ValveTarget = z.infer<typeof valveTargetSchema>;
export type HistoryPoint = z.infer<typeof historyPointSchema>;
export type ValveHistory = z.infer<typeof historySchema>;
export type PlugPowerHistory = z.infer<typeof plugPowerHistorySchema>;
export type HeatingEnergyHistory = z.infer<typeof heatingEnergyHistorySchema>;
export type HeatingEnergyPoint = HeatingEnergyHistory['points'][number];
export type Snapshot = z.infer<typeof snapshotSchema>;
export type ServerMessage = z.infer<typeof serverSchema>;
export type ControlCommand = z.infer<typeof commandSchema>;
export type ClientMessage =
  | { type: 'GetState' }
  | { type: 'Ping'; nonce: string; client_ts_ms: number }
  | { type: 'Command'; request_id: string; command: ControlCommand }
  | { type: 'GetValveHistory'; request_id: string; device: string }
  | { type: 'GetPlugPowerHistory'; request_id: string; device: string }
  | { type: 'GetHeatingEnergyHistory'; request_id: string };
