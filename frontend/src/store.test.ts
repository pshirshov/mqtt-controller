import { describe, expect, it } from 'vitest';
import fixture from '../tests/snapshot.json';
import { DashboardClient } from './store';
import { TestRuntime } from './test-runtime';
import { snapshotSchema, type ControlCommand } from './protocol';

function setup() {
  const runtime = new TestRuntime();
  const client = new DashboardClient(runtime);
  client.start();
  const socket = runtime.latest(); socket.open(); socket.pong();
  socket.receive(fixture);
  return { runtime, client, socket };
}

describe('dashboard state and commands', () => {
  it('updates the last motion report independently of current sensor state', () => {
    const { client, socket } = setup();
    const room = structuredClone(fixture.rooms[0]!);
    const sensor = room.motion_rules[0]!.sensors[0]!;
    sensor.last_event = { timestamp_epoch_ms: 1700000012345, kind: 'clear' };
    socket.receive({ type: 'Entity', kind: 'Room', data: room });
    const updated = client.getSnapshot().rooms[0]!.value.motion_rules[0]!.sensors[0]!;
    expect(updated.occupied).toBe(true);
    expect(updated.last_event).toEqual(sensor.last_event);
    client.destroy();
  });
  it.each(['ack-first', 'state-first'])('confirms a persisted motion setting with %s ordering without a device report', order => {
    const { client, socket } = setup();
    client.command('room:ensuite', { kind: 'SetMotionEnabled', room: 'ensuite', enabled: false });
    const request = socket.sent.at(-1)!;
    if (request.type !== 'Command') throw new Error('Expected a command');
    expect(client.getSnapshot().rooms[0]!.value.motion_enabled).toBe(true);
    const acknowledge = () => socket.receive({ type: 'CommandResult', request_id: request.request_id, error: null });
    const confirm = () => socket.receive({ type: 'Entity', kind: 'Room', data: {
      ...fixture.rooms[0], motion_enabled: false, target: { phase: 'unset', owner: '', since_ago_ms: null },
    } });
    if (order === 'ack-first') { acknowledge(); confirm(); }
    else { confirm(); acknowledge(); }
    expect(client.getSnapshot().commands.has('room:ensuite')).toBe(false);
    expect(client.getSnapshot().rooms[0]!.value.motion_enabled).toBe(false);
    expect(client.getSnapshot().rooms[0]!.value.actual_value).toBe('on');
    client.destroy();
  });
  it('keeps the saved motion setting on rejection and restores it from a reconnect snapshot', () => {
    const { client, socket, runtime } = setup();
    client.command('room:ensuite', { kind: 'SetMotionEnabled', room: 'ensuite', enabled: false });
    const request = socket.sent.at(-1)!;
    if (request.type !== 'Command') throw new Error('Expected a command');
    socket.receive({ type: 'CommandResult', request_id: request.request_id, error: 'Could not save motion setting' });
    expect(client.getSnapshot().rooms[0]!.value.motion_enabled).toBe(true);
    expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('error');
    socket.close(1006, 'lost');
    runtime.advance(1000);
    const replacement = runtime.latest(); replacement.open(); replacement.pong();
    replacement.receive({ ...fixture, rooms: [{ ...fixture.rooms[0], motion_enabled: false }] });
    expect(client.getSnapshot().rooms[0]!.value.motion_enabled).toBe(false);
    expect(replacement.sent.some(message => message.type === 'Command')).toBe(false);
    client.destroy();
  });
  it('handles a snapshot send failure without throwing out of the socket handler', () => {
    const runtime = new TestRuntime();
    const client = new DashboardClient(runtime);
    client.start();
    const socket = runtime.latest(); socket.open();
    socket.send = () => { throw new Error('send failed'); };
    expect(() => socket.pong()).not.toThrow();
    expect(client.getSnapshot().ready).toBe(false);
    expect(client.getSnapshot().health.lastClose!.reason).toBe('Send failed');
    client.destroy();
  });
  it('parses a complete snapshot while retaining unknown and zero values', () => {
    const snapshot = snapshotSchema.parse(fixture);
    expect(snapshot.heating_zones[0]!.trvs[1]!.battery).toBe(0);
    expect(snapshot.lights[1]!.actual_value).toBeUndefined();
  });
  it('sends an OFF command, shows acknowledgement, and waits for actual state', () => {
    const { client, socket } = setup();
    client.command('room:ensuite', { kind: 'SetRoomOff', room: 'ensuite' });
    const command = socket.sent.at(-1)!;
    expect(command.type).toBe('Command');
    if (command.type !== 'Command') throw new Error('Expected a command');
    expect(command.command).toEqual({ kind: 'SetRoomOff', room: 'ensuite' });
    expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('pending');
    socket.receive({ type: 'CommandResult', request_id: command.request_id, error: null });
    expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('accepted');
    expect(client.getSnapshot().rooms[0]!.value.actual_value).toBe('on');
    socket.receive({ type: 'Entity', kind: 'Room', data: { ...fixture.rooms[0], actual_value: 'off', physically_on: false, target_value: { kind: 'off' } } });
    expect(client.getSnapshot().rooms[0]!.value.actual_value).toBe('off');
    expect(client.getSnapshot().commands.has('room:ensuite')).toBe(false);
    client.destroy();
  });
  it.each(['ack-first', 'confirmation-first'])('dismisses a matching confirmed scene with %s ordering', order => {
    const { client, socket } = setup();
    client.command('room:ensuite', { kind: 'RecallScene', room: 'ensuite', scene_id: 2 });
    const request = socket.sent.at(-1)!;
    if (request.type !== 'Command') throw new Error('Expected a command');
    const acknowledge = () => socket.receive({ type: 'CommandResult', request_id: request.request_id, error: null });
    const confirm = () => socket.receive({ type: 'Entity', kind: 'Room', data: {
      ...fixture.rooms[0], target_value: { kind: 'on', scene_id: 2, cycle_idx: 0 },
      target: { phase: 'confirmed', owner: 'webui', since_ago_ms: 0 },
    } });
    if (order === 'ack-first') {
      acknowledge();
      expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('accepted');
      confirm();
    } else {
      confirm();
      expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('pending');
      acknowledge();
    }
    expect(client.getSnapshot().commands.has('room:ensuite')).toBe(false);
    client.destroy();
  });
  it('keeps acknowledgement for an unconfirmed target or a different confirmed scene', () => {
    const { client, socket } = setup();
    client.command('room:ensuite', { kind: 'RecallScene', room: 'ensuite', scene_id: 2 });
    const request = socket.sent.at(-1)!;
    if (request.type !== 'Command') throw new Error('Expected a command');
    socket.receive({ type: 'CommandResult', request_id: request.request_id, error: null });
    for (const [phase, scene_id] of [['commanded', 2], ['stale', 2], ['confirmed', 3]] as const) {
      socket.receive({ type: 'Entity', kind: 'Room', data: {
        ...fixture.rooms[0], target_value: { kind: 'on', scene_id, cycle_idx: 0 },
        target: { phase, owner: 'webui', since_ago_ms: 0 },
      } });
      expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('accepted');
    }
    client.destroy();
  });
  it.each([false, true])('dismisses plug power %s after a confirming snapshot, without reusing the old confirmation', on => {
    const { client, socket } = setup();
    const command: ControlCommand = { kind: 'SetPlugPower', device: 'sonoff-p-printer', on };
    client.command('plug:printer', command);
    const request = socket.sent.at(-1)!;
    if (request.type !== 'Command') throw new Error('Expected a command');
    socket.receive({ type: 'CommandResult', request_id: request.request_id, error: null });
    expect(client.getSnapshot().commands.get('plug:printer')!.state).toBe('accepted');
    socket.receive({ ...fixture, plugs: [{ ...fixture.plugs[0], target_value: on ? 'on' : 'off' }] });
    expect(client.getSnapshot().commands.has('plug:printer')).toBe(false);
    client.destroy();
  });
  it('surfaces rejection and timeout without replaying a command', () => {
    const { client, socket, runtime } = setup();
    client.command('plug:printer', { kind: 'SetPlugPower', device: 'sonoff-p-printer', on: false });
    const message = socket.sent.at(-1)!;
    if (message.type !== 'Command') throw new Error('Expected a command');
    socket.receive({ type: 'CommandResult', request_id: message.request_id, error: 'Unknown plug' });
    socket.receive({ ...fixture, plugs: [{ ...fixture.plugs[0], target_value: 'off' }] });
    expect(client.getSnapshot().commands.get('plug:printer')!.message).toBe('Unknown plug');
    client.command('plug:printer', { kind: 'SetPlugPower', device: 'sonoff-p-printer', on: false });
    runtime.advance(8000);
    socket.receive({ ...fixture, plugs: [{ ...fixture.plugs[0], target_value: 'off' }] });
    expect(client.getSnapshot().commands.get('plug:printer')!.state).toBe('error');
    expect(socket.sent.filter(message => message.type === 'Command')).toHaveLength(2);
    client.destroy();
  });
  it('requires a fresh snapshot after reconnecting and never replays pending commands', () => {
    const { client, socket, runtime } = setup();
    client.command('room:ensuite', { kind: 'SetRoomOff', room: 'ensuite' });
    socket.close(1006, 'lost');
    expect(client.getSnapshot().ready).toBe(false);
    expect(client.getSnapshot().commands.get('room:ensuite')!.state).toBe('error');
    runtime.advance(1000);
    const replacement = runtime.latest(); replacement.open(); replacement.pong();
    expect(client.getSnapshot().ready).toBe(false);
    expect(replacement.sent.some(message => message.type === 'GetState')).toBe(true);
    expect(replacement.sent.some(message => message.type === 'Command')).toBe(false);
    replacement.receive(fixture);
    expect(client.getSnapshot().ready).toBe(true);
    client.destroy();
  });
  it('bounds concurrent history queries and matches responses by request and valve', () => {
    const { client, socket, runtime } = setup();
    for (const device of ['a', 'b', 'c']) client.loadHistory(device);
    const queries = socket.sent.filter(message => message.type === 'GetValveHistory');
    expect(queries).toHaveLength(2);
    const first = queries[0]!;
    const response = { type: 'ValveHistory', request_id: first.request_id, device: first.device,
      from_epoch_ms: runtime.now() - 86400_000, to_epoch_ms: runtime.now(), points: [], error: null };
    socket.receive({ ...response, device: 'wrong' });
    expect(client.getSnapshot().histories.get('a')!.loading).toBe(true);
    socket.receive(response);
    expect(client.getSnapshot().histories.get('a')!.loading).toBe(false);
    expect(socket.sent.filter(message => message.type === 'GetValveHistory')).toHaveLength(3);
    client.destroy();
    expect(runtime.timers.size).toBe(0);
  });
  it('stores plug power history separately from valve history', () => {
    const { client, socket, runtime } = setup();
    client.loadPlugPowerHistory('sonoff-p-printer');
    const request = socket.sent.at(-1)!;
    if (request.type !== 'GetPlugPowerHistory') throw new Error('Expected a plug power history query');
    socket.receive({
      type: 'PlugPowerHistory', request_id: request.request_id, device: request.device,
      from_epoch_ms: runtime.now() - 86400_000, to_epoch_ms: runtime.now(),
      points: [{ timestamp_epoch_ms: runtime.now(), power_watts: 70, freshness: 'fresh' }],
      estimated_energy_kwh: 1.68, energy_observed_ms: 60_000, error: null,
    });
    expect(client.getSnapshot().plugHistories.get(request.device)!.data!.estimated_energy_kwh).toBe(1.68);
    expect(client.getSnapshot().histories.has(request.device)).toBe(false);
    client.destroy();
  });
  it.each([null, 0])('retains an energy estimate of %s without confusing unknown and zero', estimated_energy_kwh => {
    const { client, socket, runtime } = setup();
    client.loadPlugPowerHistory('sonoff-p-printer');
    const request = socket.sent.at(-1)!;
    if (request.type !== 'GetPlugPowerHistory') throw new Error('Expected a plug power history query');
    socket.receive({
      type: 'PlugPowerHistory', request_id: request.request_id, device: request.device,
      from_epoch_ms: runtime.now() - 86400_000, to_epoch_ms: runtime.now(), points: [],
      estimated_energy_kwh, energy_observed_ms: estimated_energy_kwh === null ? 0 : 60_000, error: null,
    });
    const history = client.getSnapshot().plugHistories.get(request.device)!;
    expect(history.loading).toBe(false);
    expect(history.data).not.toBeNull();
    expect(history.data!.estimated_energy_kwh).toBe(estimated_energy_kwh);
    client.destroy();
  });
});
