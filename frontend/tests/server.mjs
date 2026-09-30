import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { WebSocketServer } from 'ws';

const fixture = JSON.parse(await readFile(new URL('./snapshot.json', import.meta.url), 'utf8'));
const week = ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'];
fixture.heating_zones[0].trvs[0].schedule_plan = { days: Object.fromEntries(week.map(day => [day, [
  { start: '00:00', end: '06:00', temperature: 18 },
  { start: '06:00', end: '23:00', temperature: 21 },
  { start: '23:00', end: '24:00', temperature: 18 },
]])) };
const slots = (evening, other) => [
  { name: 'day', from: '06:00', to: '18:00', scene_ids: other },
  { name: 'evening', from: '18:00', to: '23:00', scene_ids: evening },
  { name: 'morning', from: '05:00', to: '06:00', scene_ids: other },
  { name: 'night', from: '23:00', to: '05:00', scene_ids: evening },
];
const scenes = [{ id: 1, name: 'bright' }, { id: 2, name: 'relaxed' }, { id: 3, name: 'dim' }];
fixture.rooms[0].schedule = { slots: slots([3, 2, 1], [1, 2]), available_scenes: scenes, overridden: false };
fixture.rooms[0].members = ['hue-l-ensuite-wall/11', 'hue-l-ensuite-ceiling/11'];
fixture.rooms[0].switch_steps = { evening: [{ scene_id: 3, lights: ['hue-l-ensuite-wall/11'] }, { scene_id: 3, lights: ['hue-l-ensuite-wall/11', 'hue-l-ensuite-ceiling/11'] }] };
fixture.rooms[0].motion_rules[0].schedule = { slots: slots([3], [1]), available_scenes: scenes, overridden: false };
fixture.rooms[0].timed_actions = [{ binding: 'ensuite-night-off', time: '23:30', action: 'turn_off → ensuite', overridden: false }];
fixture.plugs[0].schedule = { timed_actions: [{ time: '07:00', action: 'on' }], kill_switch: { threshold_watts: 10, holdoff_secs: 120 }, overridden: false, power_metered: true };
fixture.plugs[0].kill_switch_rules[0].overridden = false;
const root = resolve('dist');
const server = createServer(async (request, response) => {
  const path = new URL(request.url, 'http://localhost').pathname;
  const file = resolve(root, '.' + (path === '/' ? '/index.html' : path));
  if (!file.startsWith(root + '/')) { response.writeHead(403).end(); return; }
  try {
    const content = await readFile(file);
    response.setHeader('Content-Type', { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css' }[extname(file)] ?? 'application/octet-stream');
    response.end(content);
  } catch { response.writeHead(404).end(); }
});
const wss = new WebSocketServer({ server, path: '/ws' });
wss.on('connection', socket => {
  const state = structuredClone(fixture);
  const send = message => socket.send(JSON.stringify(message));
  send(state);
  socket.on('message', data => {
    const message = JSON.parse(data.toString());
    if (message.type === 'Ping') send({ type: 'Pong', nonce: message.nonce, client_ts_ms: message.client_ts_ms, server_ts_ms: Date.now() });
    if (message.type === 'GetState') send(state);
    if (message.type === 'Command') {
      const command = message.command;
      send({ type: 'CommandResult', request_id: message.request_id, error: null });
      if (command.kind === 'SetRoomOff' || command.kind === 'RecallScene') {
        const room = state.rooms.find(room => room.name === command.room);
        room.target_value = command.kind === 'SetRoomOff' ? { kind: 'off' } : { kind: 'on', scene_id: command.scene_id, cycle_idx: 0 };
        room.target = { phase: 'commanded', owner: 'webui', since_ago_ms: 0 };
        send({ type: 'Entity', kind: 'Room', data: room });
        setTimeout(() => {
          room.actual_value = command.kind === 'SetRoomOff' ? 'off' : 'on'; room.physically_on = room.actual_value === 'on';
          room.target.phase = 'confirmed';
          send({ type: 'Entity', kind: 'Room', data: room });
        }, 100);
      } else if (command.kind === 'SetMotionEnabled') {
        const room = state.rooms.find(room => room.name === command.room);
        room.motion_enabled = command.enabled;
        send({ type: 'Entity', kind: 'Room', data: room });
      } else if (command.kind === 'SetHeatDemandEnabled') {
        const zone = state.heating_zones.find(zone => zone.trvs.some(valve => valve.device === command.device));
        zone.trvs.find(valve => valve.device === command.device).heat_demand_enabled = command.enabled;
        send({ type: 'Entity', kind: 'HeatingZone', data: zone });
      } else if (command.kind === 'StartValveBoost' || command.kind === 'SetValveBoostTarget' || command.kind === 'CancelValveBoost') {
        const zone = state.heating_zones.find(zone => zone.trvs.some(valve => valve.device === command.device));
        const valve = zone.trvs.find(valve => valve.device === command.device);
        if (command.kind === 'StartValveBoost') valve.boost = {
          temperature: command.temperature, ends_at_epoch_ms: Date.now() + command.duration_minutes * 60_000,
          remaining_ms: command.duration_minutes * 60_000,
        };
        else if (command.kind === 'SetValveBoostTarget') valve.boost = {
          ...valve.boost, temperature: command.temperature, remaining_ms: Math.max(0, valve.boost.ends_at_epoch_ms - Date.now()),
        };
        else valve.boost = null;
        send({ type: 'Entity', kind: 'HeatingZone', data: zone });
      } else if (command.kind === 'SetValveSchedule' || command.kind === 'ResetValveSchedule') {
        const zone = state.heating_zones.find(zone => zone.trvs.some(valve => valve.device === command.device));
        const valve = zone.trvs.find(valve => valve.device === command.device);
        valve.schedule_override = command.kind === 'SetValveSchedule';
        valve.schedule_plan = command.kind === 'SetValveSchedule' ? command.schedule : fixture.heating_zones[0].trvs[0].schedule_plan;
        send({ type: 'Entity', kind: 'HeatingZone', data: zone });
      } else if (command.kind === 'SetRoomSchedule' || command.kind === 'ResetRoomSchedule') {
        const room = state.rooms.find(room => room.name === command.room);
        room.schedule = command.kind === 'SetRoomSchedule' ? { ...room.schedule, slots: command.slots, overridden: true } : structuredClone(fixture.rooms[0].schedule);
        room.switch_steps = command.kind === 'SetRoomSchedule' ? command.switch_steps : structuredClone(fixture.rooms[0].switch_steps);
        const active = room.schedule.slots.find(slot => slot.name === room.active_slot) ?? room.schedule.slots[0];
        room.active_slot = active.name;
        room.scene_ids = active.scene_ids;
        send({ type: 'Entity', kind: 'Room', data: room });
      } else if (command.kind === 'SetMotionSchedule' || command.kind === 'ResetMotionSchedule') {
        const room = state.rooms.find(room => room.motion_rules.some(rule => rule.name === command.rule));
        const rule = room.motion_rules.find(rule => rule.name === command.rule);
        rule.schedule = command.kind === 'SetMotionSchedule' ? { ...rule.schedule, slots: command.slots, overridden: true } : structuredClone(fixture.rooms[0].motion_rules[0].schedule);
        send({ type: 'Entity', kind: 'Room', data: room });
      } else if (command.kind === 'SetTimedActionTime' || command.kind === 'ResetTimedActionTime') {
        const entities = state.rooms.map(data => ({ kind: 'Room', data }));
        const entity = entities.find(entity => entity.data.timed_actions.some(action => action.binding === command.binding));
        const deployed = fixture.rooms.flatMap(item => item.timed_actions).find(action => action.binding === command.binding);
        entity.data.timed_actions = entity.data.timed_actions.map(action => action.binding !== command.binding ? action
          : command.kind === 'SetTimedActionTime' ? { ...action, time: command.time, overridden: true } : { ...deployed });
        send({ type: 'Entity', ...entity });
      } else if (command.kind === 'SetPlugSchedule' || command.kind === 'ResetPlugSchedule') {
        const plug = state.plugs.find(plug => plug.device === command.device);
        const deployed = fixture.plugs.find(plug => plug.device === command.device);
        plug.schedule = command.kind === 'SetPlugSchedule'
          ? { ...plug.schedule, timed_actions: command.timed_actions, kill_switch: command.kill_switch, overridden: true } : structuredClone(deployed.schedule);
        plug.kill_switch_rules = plug.schedule.kill_switch === null ? [] : [{
          rule_name: command.kind === 'SetPlugSchedule' ? `plug-schedule:${plug.device}:kill-switch` : deployed.kill_switch_rules[0].rule_name,
          state: 'armed', threshold_watts: plug.schedule.kill_switch.threshold_watts, holdoff_secs: plug.schedule.kill_switch.holdoff_secs, overridden: plug.schedule.overridden,
        }];
        send({ type: 'Entity', kind: 'Plug', data: plug });
      } else if (command.kind === 'SetPlugPower') {
        const plug = state.plugs.find(plug => plug.device === command.device);
        plug.target_value = command.on ? 'on' : 'off'; plug.actual_value.on = command.on;
        send({ type: 'Entity', kind: 'Plug', data: plug });
      }
    }
    if (message.type === 'GetHeatingEnergyHistory') {
      const end = Date.now();
      const points = Array.from({ length: 1441 }, (_, index) => ({
        timestamp_epoch_ms: end - (1440 - index) * 60_000,
        relays: [
          { zone: state.heating_zones[0].name, device: 'bosch-wt-master-bedroom-wall', on: index >= 120 && index < 180, freshness: 'fresh' },
          { zone: 'downstairs', device: 'bosch-wt-kitchen-wall', on: index >= 150 && index < 210, freshness: 'fresh' },
        ],
        heat_pump: { device: 'nodon-mtr-heat-pump', power_watts: index >= 120 && index < 210 ? 3000 : 50,
          energy_kwh: 100 + index / 120, power_freshness: 'fresh', energy_freshness: 'fresh' },
      }));
      send({ type: 'HeatingEnergyHistory', request_id: message.request_id, from_epoch_ms: end - 86400_000, to_epoch_ms: end, points, error: null });
    }
    if (message.type === 'GetValveHistory') {
      const end = Date.now();
      const points = Array.from({ length: 1440 }, (_, index) => {
        const heatingDemand = Math.round(40 + 35 * Math.sin(index / 90));
        const sonoffHeating = index >= 900 && index < 1100;
        return {
          timestamp_epoch_ms: end - (1440 - index) * 60_000, observed_at_epoch_ms: end - (1440 - index) * 60_000 - 500,
          local_temperature: index > 200 && index < 250 ? null : 20 + Math.sin(index / 90),
          reported_setpoint: index < 700 ? 18 : 21, target: { kind: 'setpoint', temperature: index < 700 ? 18 : 21 },
          heating_demand: message.device === 'sonoff-trv-ensuite' ? null : heatingDemand,
          running_state: message.device === 'sonoff-trv-ensuite' ? sonoffHeating ? 'heat' : 'idle' : heatingDemand >= 40 ? 'heat' : 'idle',
          battery: 80, freshness: 'fresh',
        };
      });
      send({ type: 'ValveHistory', request_id: message.request_id, device: message.device, from_epoch_ms: end - 86400_000, to_epoch_ms: end, points, error: null });
    }
    if (message.type === 'GetPlugPowerHistory') {
      const end = Date.now();
      const points = Array.from({ length: 1440 }, (_, index) => ({
        timestamp_epoch_ms: end - (1440 - index) * 60_000,
        power_watts: Math.max(0, 70 + 30 * Math.sin(index / 90)), freshness: 'fresh',
      }));
      send({ type: 'PlugPowerHistory', request_id: message.request_id, device: message.device,
        from_epoch_ms: end - 86400_000, to_epoch_ms: end, points, estimated_energy_kwh: 1.68, energy_observed_ms: 1439 * 60_000, error: null });
    }
  });
});
server.listen(18780, '127.0.0.1');
