import { expect, it } from 'vitest';
import { aggregateSeriesLabel, chartSegments, sensorColor } from './HistoryChart';
import type { HistoryPoint, ValveHistory } from './protocol';

function point(timestamp_epoch_ms: number, local_temperature: number | null): HistoryPoint {
  return { timestamp_epoch_ms, local_temperature, observed_at_epoch_ms: timestamp_epoch_ms, remote_temperature: null,
    target: null, reported_setpoint: null, heating_demand: null, running_state: 'unknown', battery: null, freshness: 'fresh' };
}
it('leaves gaps for missing telemetry and controller downtime', () => {
  const path = chartSegments([point(0, 20), point(60_000, 21), point(120_000, null), point(180_000, 22), point(400_000, 23)],
    point => point.local_temperature, time => time / 1000, temperature => temperature, false);
  expect(path).toBe('M0.00,20.00L60.00,21.00M180.00,22.00M400.00,23.00');
});

it('assigns stable distinct colors beyond a fixed six-series palette', () => {
  const devices = Array.from({ length: 8 }, (_, index) => `sensor-${index}`);
  expect(new Set(devices.map(sensorColor)).size).toBe(devices.length);
  expect(sensorColor(devices[0]!)).toBe(sensorColor(devices[0]!));
});

it('breaks aggregate lines and labels when the configured policy changes', () => {
  const points = [
    ['average', 20], ['average', 21], ['freshest', 22], ['freshest', 23],
  ].map(([policy, aggregate], index) => ({
    ...point(index * 60_000, aggregate as number),
    remote_temperature: { policy, aggregate_temperature: aggregate, sources: [] },
  } as HistoryPoint));
  const path = chartSegments(points, item => item.remote_temperature?.aggregate_temperature ?? null,
    time => time / 1000, value => value, false, item => item.remote_temperature?.policy ?? null);
  expect(path).toBe('M0.00,20.00L60.00,21.00M120.00,22.00L180.00,23.00');
  expect(aggregateSeriesLabel({ points } as ValveHistory)).toBe('Aggregate · policy changed');
});
