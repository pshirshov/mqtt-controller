import { ConnectionManager, type Health, type Runtime } from './connection';
import type { ControlCommand, HeatingEnergyHistory, HeatingZone, Light, Plug, PlugPowerHistory, Room, ServerMessage, ValveHistory } from './protocol';

export interface Timed<T> { value: T; receivedAt: number }
export type CommandStatus =
  | { state: 'pending' | 'accepted'; message: string; command: ControlCommand; confirmed: boolean }
  | { state: 'error'; message: string };
export interface HistoryStatus<T = ValveHistory> { loading: boolean; data: T | null; error: string | null }
export interface DashboardState {
  health: Health; ready: boolean; receivedAt: number | null;
  rooms: Timed<Room>[]; plugs: Timed<Plug>[]; lights: Timed<Light>[]; heating: Timed<HeatingZone>[];
  commands: ReadonlyMap<string, CommandStatus>; histories: ReadonlyMap<string, HistoryStatus>;
  plugHistories: ReadonlyMap<string, HistoryStatus<PlugPowerHistory>>;
  heatingEnergy: HistoryStatus<HeatingEnergyHistory>;
}
type HistoryRequest = { kind: 'valve' | 'plug'; device: string } | { kind: 'heatingEnergy' };
interface AnyHistoryStatus { loading: boolean; data: ValveHistory | PlugPowerHistory | HeatingEnergyHistory | null; error: string | null }
const COMMAND_TIMEOUT_MS = 8_000;
const HISTORY_TIMEOUT_MS = 10_000;
const HISTORY_CONCURRENCY = 2;

export class DashboardClient {
  readonly connection: ConnectionManager;
  private state: DashboardState;
  private readonly subscribers = new Set<() => void>();
  private readonly pendingCommands = new Map<string, { key: string; timer: number }>();
  private readonly pendingHistories = new Map<string, HistoryRequest & { timer: number }>();
  private historyQueue: HistoryRequest[] = [];
  private destroyed = false;

  constructor(private readonly runtime: Runtime) {
    this.connection = new ConnectionManager(runtime,
      health => this.healthChanged(health), message => this.receive(message), () => this.resync());
    this.state = {
      health: this.connection.stats(), ready: false, receivedAt: null,
      rooms: [], plugs: [], lights: [], heating: [], commands: new Map(), histories: new Map(), plugHistories: new Map(),
      heatingEnergy: { loading: false, data: null, error: null },
    };
  }
  getSnapshot = (): DashboardState => this.state;
  subscribe = (listener: () => void): (() => void) => { this.subscribers.add(listener); return () => this.subscribers.delete(listener); };
  start(): void { this.connection.start(); }
  destroy(): void {
    this.destroyed = true;
    for (const command of this.pendingCommands.values()) this.runtime.cancel(command.timer);
    for (const history of this.pendingHistories.values()) this.runtime.cancel(history.timer);
    this.pendingCommands.clear();
    this.pendingHistories.clear();
    this.subscribers.clear();
    this.connection.destroy();
  }

  command(key: string, command: ControlCommand): void {
    if (this.destroyed) return;
    const current = this.state.commands.get(key);
    if (current !== undefined && current.state === 'pending') return;
    if (!this.state.ready) { this.commandStatus(key, { state: 'error', message: 'Waiting for a live controller snapshot. No command was sent.' }); return; }
    const request_id = this.runtime.nonce();
    const timer = this.runtime.later(() => {
      this.pendingCommands.delete(request_id);
      this.commandStatus(key, { state: 'error', message: 'No acknowledgement. Check the reported state before trying again.' });
    }, COMMAND_TIMEOUT_MS);
    this.pendingCommands.set(request_id, { key, timer });
    this.commandStatus(key, { state: 'pending', message: 'Sending command…', command, confirmed: false });
    try { this.connection.send({ type: 'Command', request_id, command }); }
    catch (error) {
      this.runtime.cancel(timer);
      this.pendingCommands.delete(request_id);
      this.commandStatus(key, { state: 'error', message: error instanceof Error ? error.message : 'Sending failed' });
    }
  }

  loadHistory(device: string): void {
    this.queueHistory({ kind: 'valve', device });
  }

  loadPlugPowerHistory(device: string): void {
    this.queueHistory({ kind: 'plug', device });
  }

  loadHeatingEnergy(): void { this.queueHistory({ kind: 'heatingEnergy' }); }

  private existingHistory(request: HistoryRequest): AnyHistoryStatus | undefined {
    if (request.kind === 'heatingEnergy') return this.state.heatingEnergy;
    return request.kind === 'valve' ? this.state.histories.get(request.device) : this.state.plugHistories.get(request.device);
  }

  private queueHistory(request: HistoryRequest): void {
    if (this.destroyed || !this.state.ready) return;
    const same = (other: HistoryRequest) => other.kind === request.kind && (other.kind === 'heatingEnergy'
      || (request.kind !== 'heatingEnergy' && other.device === request.device));
    if (this.historyQueue.some(same) || [...this.pendingHistories.values()].some(same)) return;
    const existing = this.existingHistory(request);
    this.historyStatus(request, { loading: true, data: existing === undefined ? null : existing.data, error: null });
    this.historyQueue.push(request);
    this.pumpHistory();
  }

  private publish(update: Partial<DashboardState>): void {
    if (this.destroyed) return;
    this.state = { ...this.state, ...update };
    for (const listener of this.subscribers) listener();
  }

  private healthChanged(health: Health): void {
    const alive = health.connections.some(connection => connection.id === health.activeId && connection.state === 'ALIVE');
    if (!alive) {
      for (const [request, pending] of this.pendingCommands) {
        this.runtime.cancel(pending.timer);
        this.pendingCommands.delete(request);
        this.commandStatus(pending.key, { state: 'error', message: 'Connection interrupted. The command outcome is unknown; check reported state after reconnecting.' });
      }
      for (const pending of this.pendingHistories.values()) {
        this.runtime.cancel(pending.timer);
        const existing = this.existingHistory(pending);
        this.historyStatus(pending, { loading: false, data: existing === undefined ? null : existing.data, error: 'History will refresh when the connection recovers.' });
      }
      this.pendingHistories.clear();
      for (const request of this.historyQueue) {
        const existing = this.existingHistory(request);
        this.historyStatus(request, { loading: false, data: existing === undefined ? null : existing.data, error: 'Waiting for connection' });
      }
      this.historyQueue = [];
    }
    this.publish({ health, ready: alive && this.state.ready });
  }

  private resync(): void {
    this.publish({ ready: false });
    try { this.connection.send({ type: 'GetState' }); }
    catch { /* The connection manager exposes the failure and schedules recovery. */ }
  }

  private commandStatus(key: string, status: CommandStatus | undefined): void {
    const commands = new Map(this.state.commands);
    if (status === undefined) commands.delete(key);
    else commands.set(key, status);
    this.publish({ commands });
  }

  // Only subsequent entity updates can confirm a submitted command; cached
  // confirmation may belong to a previous request for the same target.
  private confirmTargets(rooms: Room[], plugs: Plug[], zones: HeatingZone[]): ReadonlyMap<string, CommandStatus> {
    const commands = new Map(this.state.commands);
    for (const [key, status] of commands) {
      if (status.state === 'error') continue;
      const command = status.command;
      let confirmed: boolean;
      if (command.kind === 'SetPlugPower') {
        const plug = plugs.find(plug => plug.device === command.device);
        if (plug === undefined) continue;
        confirmed = plug.target != null && plug.target.phase === 'confirmed'
          && plug.target_value === (command.on ? 'on' : 'off');
      } else if (command.kind === 'SetHeatDemandEnabled') {
        const valve = zones.flatMap(zone => zone.trvs).find(valve => valve.device === command.device);
        if (valve === undefined) continue;
        confirmed = valve.heat_demand_enabled === command.enabled;
      } else if (command.kind === 'StartValveBoost' || command.kind === 'SetValveBoostTarget' || command.kind === 'CancelValveBoost') {
        const valve = zones.flatMap(zone => zone.trvs).find(valve => valve.device === command.device);
        if (valve === undefined) continue;
        confirmed = command.kind === 'CancelValveBoost' ? valve.boost === null
          : valve.boost !== null && valve.boost.temperature === command.temperature;
      } else {
        const room = rooms.find(room => room.name === command.room);
        if (room === undefined) continue;
        const target = room.target_value;
        confirmed = command.kind === 'SetMotionEnabled' ? room.motion_enabled === command.enabled
          : room.target != null && room.target.phase === 'confirmed' && target != null
          && (command.kind === 'SetRoomOff' ? target.kind === 'off'
            : target.kind === 'on' && target.scene_id === command.scene_id);
      }
      if (status.state === 'accepted' && confirmed) commands.delete(key);
      else if (confirmed !== status.confirmed) commands.set(key, { ...status, confirmed });
    }
    return commands;
  }

  private historyStatus(request: HistoryRequest, status: AnyHistoryStatus): void {
    if (request.kind === 'heatingEnergy') {
      this.publish({ heatingEnergy: status as HistoryStatus<HeatingEnergyHistory> });
    } else if (request.kind === 'valve') {
      const histories = new Map(this.state.histories);
      histories.set(request.device, status as HistoryStatus);
      this.publish({ histories });
    } else {
      const plugHistories = new Map(this.state.plugHistories);
      plugHistories.set(request.device, status as HistoryStatus<PlugPowerHistory>);
      this.publish({ plugHistories });
    }
  }

  private pumpHistory(): void {
    while (this.state.ready && this.pendingHistories.size < HISTORY_CONCURRENCY && this.historyQueue.length > 0) {
      const request = this.historyQueue.shift();
      if (request === undefined) throw new Error('History queue invariant violated');
      const request_id = this.runtime.nonce();
      const timer = this.runtime.later(() => {
        this.pendingHistories.delete(request_id);
        const existing = this.existingHistory(request);
        this.historyStatus(request, { loading: false, data: existing === undefined ? null : existing.data, error: 'History request timed out. Try again.' });
        this.pumpHistory();
      }, HISTORY_TIMEOUT_MS);
      this.pendingHistories.set(request_id, { ...request, timer });
      try { this.connection.send(request.kind === 'heatingEnergy' ? { type: 'GetHeatingEnergyHistory', request_id } : request.kind === 'valve'
        ? { type: 'GetValveHistory', request_id, device: request.device }
        : { type: 'GetPlugPowerHistory', request_id, device: request.device }); }
      catch {
        this.runtime.cancel(timer);
        this.pendingHistories.delete(request_id);
        const existing = this.existingHistory(request);
        this.historyStatus(request, { loading: false, data: existing === undefined ? null : existing.data, error: 'Could not request history. Try again when connected.' });
      }
    }
  }

  private receive(message: ServerMessage): void {
    const receivedAt = this.runtime.now();
    switch (message.type) {
      case 'StateSnapshot':
        this.publish({
          ready: true, receivedAt,
          rooms: message.rooms.map(value => ({ value, receivedAt })), plugs: message.plugs.map(value => ({ value, receivedAt })),
          lights: message.lights.map(value => ({ value, receivedAt })), heating: message.heating_zones.map(value => ({ value, receivedAt })),
          commands: this.confirmTargets(message.rooms, message.plugs, message.heating_zones),
        });
        break;
      case 'Entity':
        if (!this.state.ready) break;
        switch (message.kind) {
          case 'Room': this.publish({ rooms: replace(this.state.rooms, message.data, receivedAt, room => room.name), commands: this.confirmTargets([message.data], [], []) }); break;
          case 'Plug': this.publish({ plugs: replace(this.state.plugs, message.data, receivedAt, plug => plug.device), commands: this.confirmTargets([], [message.data], []) }); break;
          case 'Light': this.publish({ lights: replace(this.state.lights, message.data, receivedAt, light => light.device) }); break;
          case 'HeatingZone': this.publish({ heating: replace(this.state.heating, message.data, receivedAt, zone => zone.name), commands: this.confirmTargets([], [], [message.data]) }); break;
        }
        break;
      case 'CommandResult': {
        const pending = this.pendingCommands.get(message.request_id);
        if (pending === undefined) break;
        this.runtime.cancel(pending.timer);
        this.pendingCommands.delete(message.request_id);
        const status = this.state.commands.get(pending.key);
        if (status === undefined || status.state !== 'pending') throw new Error('Pending command status invariant violated');
        this.commandStatus(pending.key, message.error === null
          ? status.confirmed ? undefined : { ...status, state: 'accepted', message: status.command.kind === 'SetMotionEnabled' || status.command.kind === 'SetHeatDemandEnabled'
            || status.command.kind === 'StartValveBoost' || status.command.kind === 'SetValveBoostTarget' || status.command.kind === 'CancelValveBoost'
            ? 'Setting saved. Waiting for updated controller state.' : 'Command accepted. Reported state updates when the device responds.' }
          : { state: 'error', message: message.error });
        break;
      }
      case 'HeatingEnergyHistory': {
        const pending = this.pendingHistories.get(message.request_id);
        if (pending === undefined || pending.kind !== 'heatingEnergy') break;
        this.runtime.cancel(pending.timer);
        this.pendingHistories.delete(message.request_id);
        this.historyStatus(pending, { loading: false, data: message, error: message.error });
        this.pumpHistory();
        break;
      }
      case 'ValveHistory': {
        const pending = this.pendingHistories.get(message.request_id);
        if (pending === undefined || pending.kind !== 'valve' || pending.device !== message.device) break;
        this.runtime.cancel(pending.timer);
        this.pendingHistories.delete(message.request_id);
        this.historyStatus(pending, { loading: false, data: message, error: message.error });
        this.pumpHistory();
        break;
      }
      case 'PlugPowerHistory': {
        const pending = this.pendingHistories.get(message.request_id);
        if (pending === undefined || pending.kind !== 'plug' || pending.device !== message.device) break;
        this.runtime.cancel(pending.timer);
        this.pendingHistories.delete(message.request_id);
        this.historyStatus(pending, { loading: false, data: message, error: message.error });
        this.pumpHistory();
        break;
      }
      case 'Pong': case 'EventLog': case 'EntityLog': case 'Topology': break;
    }
  }
}

function replace<T>(items: Timed<T>[], value: T, receivedAt: number, key: (value: T) => string): Timed<T>[] {
  const index = items.findIndex(item => key(item.value) === key(value));
  const next = [...items];
  if (index < 0) next.push({ value, receivedAt });
  else next[index] = { value, receivedAt };
  return next;
}
