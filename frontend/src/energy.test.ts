import { expect, it } from 'vitest';
import { heatPumpHistory, relayHistory, relayTotals } from './energy';
import type { HeatingEnergyHistory, HeatingEnergyPoint } from './protocol';

function point(minute: number, upstairs: boolean | null, downstairs: boolean | null, energy: number | null): HeatingEnergyPoint {
  return { timestamp_epoch_ms: minute * 60_000, relays: [
    { zone: 'upstairs', device: 'relay-u', on: upstairs, freshness: upstairs === null ? 'unknown' : 'fresh' },
    { zone: 'downstairs', device: 'relay-d', on: downstairs, freshness: downstairs === null ? 'unknown' : 'fresh' },
  ], heat_pump: { device: 'meter', power_watts: 1000, energy_kwh: energy, energy_freshness: energy === null ? 'unknown' : 'fresh', power_freshness: 'fresh' } };
}
function history(points: HeatingEnergyPoint[]): HeatingEnergyHistory {
  return { type: 'HeatingEnergyHistory', request_id: 'energy', from_epoch_ms: 0, to_epoch_ms: 86400_000, points, error: null };
}

// Specified: summed zone-hours differ from elapsed time when zones overlap.
it('counts reported relay intervals and their union without double-counting overlap', () => {
  const data = history([point(0, true, false, 100), point(1, true, true, 101), point(2, false, true, 102), point(3, false, false, 103)]);
  const upstairs = relayHistory(data, 'upstairs', 'relay-u');
  const downstairs = relayHistory(data, 'downstairs', 'relay-d');
  expect(upstairs.on_ms).toBe(120_000);
  expect(downstairs.on_ms).toBe(120_000);
  expect(upstairs.observed_ms).toBe(180_000);
  expect(relayTotals([upstairs, downstairs])).toEqual({ summed_ms: 240_000, any_on_ms: 180_000 });
});

it('keeps stale readings, absent relays and sample gaps out of runtime totals', () => {
  const stale = point(2, true, true, 102);
  stale.relays.forEach(relay => { relay.freshness = 'stale'; });
  const data = history([point(0, true, null, 100), point(1, false, null, 101), stale, point(10, true, null, 103)]);
  expect(relayHistory(data, 'upstairs', 'relay-u').on_ms).toBe(60_000);
  expect(relayHistory(data, 'upstairs', 'relay-u').observed_ms).toBe(60_000);
  expect(relayHistory(data, 'downstairs', 'relay-d').on_ms).toBeNull();
  expect(relayHistory(data, 'upstairs', 'replacement-relay').on_ms).toBeNull();
  expect(relayTotals([])).toEqual({ summed_ms: null, any_on_ms: null });
});

it('distinguishes measured OFF from insufficient relay data', () => {
  const data = history([point(0, false, null, 100), point(1, false, null, 100)]);
  expect(relayHistory(data, 'upstairs', 'relay-u').on_ms).toBe(0);
  expect(relayHistory(history([data.points[0]!]), 'upstairs', 'relay-u').on_ms).toBeNull();
});

it('measures counter consumption across reporting gaps and exposes counter resets', () => {
  const data = history([point(0, true, true, 100), point(1, true, true, null), point(120, true, true, 104),
    point(121, true, true, 0), point(180, true, true, 1.5)]);
  const pump = heatPumpHistory(data);
  expect(pump).toMatchObject({ consumed_energy_kwh: 5.5, observed_ms: 179 * 60_000, resets: 1 });
});

it('does not bridge meter replacements or invent zero consumption from missing data', () => {
  const replaced = point(1, false, false, 10);
  if (replaced.heat_pump === null) throw new Error('fixture meter missing');
  replaced.heat_pump.device = 'replacement';
  expect(heatPumpHistory(history([point(0, false, false, 100), replaced]))).toMatchObject({ consumed_energy_kwh: null, observed_ms: 0 });
  expect(heatPumpHistory(history([]))).toBeNull();
  expect(heatPumpHistory(history([point(0, false, false, 100), point(1, false, false, 100)]))).toMatchObject({ consumed_energy_kwh: 0, observed_ms: 60_000 });
});
