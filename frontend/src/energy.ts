import type { HeatingEnergyHistory, Plug, PlugPowerHistory } from './protocol';
import type { HistoryStatus, Timed } from './store';

export const MAX_SAMPLE_GAP_MS = 90_000;
export interface Interval { from: number; to: number }
export interface RelayPoint { timestamp_epoch_ms: number; on: boolean | null; freshness: string }
export interface RelayHistory {
  from_epoch_ms: number; to_epoch_ms: number; points: RelayPoint[];
  active: Interval[]; on_ms: number | null; observed_ms: number;
}

export function totalPlugEnergyKwh(plugs: Timed<Plug>[], histories: ReadonlyMap<string, HistoryStatus<PlugPowerHistory>>): number | null {
  let total: number | null = null;
  for (const plug of plugs) {
    if (plug.value.exclude_from_totals) continue;
    const history = histories.get(plug.value.device);
    if (history === undefined || history.data === null || history.data.estimated_energy_kwh === null) continue;
    total = (total ?? 0) + history.data.estimated_energy_kwh;
  }
  return total;
}

export function relayHistory(history: HeatingEnergyHistory, zone: string, device: string): RelayHistory {
  const points = history.points.map(point => {
    const relay = point.relays.find(relay => relay.zone === zone && relay.device === device);
    return { timestamp_epoch_ms: point.timestamp_epoch_ms, on: relay === undefined ? null : relay.on,
      freshness: relay === undefined ? 'unknown' : relay.freshness };
  });
  let observed_ms = 0;
  let on_ms = 0;
  const active: Interval[] = [];
  for (const [index, current] of points.entries()) {
    const previous = points[index - 1];
    if (previous === undefined) continue;
    const elapsed = current.timestamp_epoch_ms - previous.timestamp_epoch_ms;
    if (elapsed <= 0 || elapsed > MAX_SAMPLE_GAP_MS || previous.freshness !== 'fresh' || current.freshness !== 'fresh'
      || previous.on === null || current.on === null) continue;
    observed_ms += elapsed;
    if (previous.on) {
      on_ms += elapsed;
      const last = active.at(-1);
      if (last !== undefined && last.to === previous.timestamp_epoch_ms) last.to = current.timestamp_epoch_ms;
      else active.push({ from: previous.timestamp_epoch_ms, to: current.timestamp_epoch_ms });
    }
  }
  return { from_epoch_ms: history.from_epoch_ms, to_epoch_ms: history.to_epoch_ms, points, active,
    on_ms: observed_ms === 0 ? null : on_ms, observed_ms };
}

export function relayTotals(histories: RelayHistory[]): { summed_ms: number | null; any_on_ms: number | null } {
  const known = histories.filter(history => history.on_ms !== null);
  if (known.length === 0) return { summed_ms: null, any_on_ms: null };
  const intervals = known.flatMap(history => history.active).sort((a, b) => a.from - b.from);
  let any_on_ms = 0;
  let until = -Infinity;
  for (const interval of intervals) {
    any_on_ms += Math.max(0, interval.to - Math.max(until, interval.from));
    until = Math.max(until, interval.to);
  }
  return { summed_ms: known.reduce((total, history) => total + (history.on_ms ?? 0), 0), any_on_ms };
}

export function heatPumpHistory(history: HeatingEnergyHistory) {
  const latest = [...history.points].reverse().find(point => point.heat_pump !== null);
  if (latest === undefined || latest.heat_pump === null) return null;
  const device = latest.heat_pump.device;
  const points = history.points.map(point => ({ timestamp_epoch_ms: point.timestamp_epoch_ms,
    power_watts: point.heat_pump !== null && point.heat_pump.device === device ? point.heat_pump.power_watts : null,
    freshness: point.heat_pump !== null && point.heat_pump.device === device ? point.heat_pump.power_freshness : 'unknown',
  }));
  let previous: { timestamp: number; energy: number } | null = null;
  let kwh: number | null = null;
  let observed_ms = 0;
  let resets = 0;
  for (const point of history.points) {
    const meter = point.heat_pump;
    if (meter === null || meter.device !== device) { previous = null; continue; }
    if (meter.energy_freshness !== 'fresh' || meter.energy_kwh === null) continue;
    if (previous !== null && point.timestamp_epoch_ms > previous.timestamp) {
      const delta = meter.energy_kwh - previous.energy;
      if (delta < 0) resets++;
      else { kwh = (kwh ?? 0) + delta; observed_ms += point.timestamp_epoch_ms - previous.timestamp; }
    }
    previous = { timestamp: point.timestamp_epoch_ms, energy: meter.energy_kwh };
  }
  return { device, points, from_epoch_ms: history.from_epoch_ms, to_epoch_ms: history.to_epoch_ms,
    consumed_energy_kwh: kwh, observed_ms, resets };
}
