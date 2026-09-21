import { useEffect, useMemo, useRef, useState } from 'react';
import type { PlugPowerHistory, ValveHistory } from './protocol';
import { temperature, valveTarget } from './format';

const WIDTH = 480;
const HEIGHT = 190;
const LEFT = 50;
const RIGHT = WIDTH - 10;
const TOP = 12;
const BOTTOM = HEIGHT - 24;
const MAX_SAMPLE_GAP_MS = 90_000;

interface Timestamped { timestamp_epoch_ms: number }
interface HeatingRegion { from: number; to: number }

export function chartSegments<T extends Timestamped>(
  points: T[], value: (point: T) => number | null, x: (timestamp: number) => number,
  y: (temperature: number) => number, step: boolean,
): string {
  let path = '';
  let previous: T | null = null;
  for (const point of points) {
    const current = value(point);
    if (current === null) { previous = null; continue; }
    const xpos = x(point.timestamp_epoch_ms).toFixed(2);
    const ypos = y(current).toFixed(2);
    if (previous === null || point.timestamp_epoch_ms - previous.timestamp_epoch_ms > MAX_SAMPLE_GAP_MS) path += `M${xpos},${ypos}`;
    else path += step ? `H${xpos}V${ypos}` : `L${xpos},${ypos}`;
    previous = point;
  }
  return path;
}

function heatingRegions(history: ValveHistory): HeatingRegion[] {
  const regions: HeatingRegion[] = [];
  for (const [index, point] of history.points.entries()) {
    if (point.freshness !== 'fresh' || point.running_state !== 'heat') continue;
    const next = history.points[index + 1];
    const to = next !== undefined && next.timestamp_epoch_ms > point.timestamp_epoch_ms
      && next.timestamp_epoch_ms - point.timestamp_epoch_ms <= MAX_SAMPLE_GAP_MS
      ? next.timestamp_epoch_ms
      : index === history.points.length - 1 && history.to_epoch_ms > point.timestamp_epoch_ms
        && history.to_epoch_ms - point.timestamp_epoch_ms <= MAX_SAMPLE_GAP_MS
        ? history.to_epoch_ms : point.timestamp_epoch_ms;
    if (to === point.timestamp_epoch_ms) continue;
    const previous = regions.at(-1);
    if (previous !== undefined && previous.to === point.timestamp_epoch_ms) previous.to = to;
    else regions.push({ from: point.timestamp_epoch_ms, to });
  }
  return regions;
}

function nearest<T extends Timestamped>(points: T[], timestamp: number | null): T | undefined {
  if (timestamp === null) return undefined;
  return points.reduce<T | undefined>((candidate, point) =>
    candidate === undefined || Math.abs(point.timestamp_epoch_ms - timestamp) < Math.abs(candidate.timestamp_epoch_ms - timestamp)
      ? point : candidate, undefined);
}

function useHistoryInspection<T extends Timestamped>(points: T[], from: number, to: number) {
  const [hover, setHover] = useState<number | null>(null);
  const [pinned, setPinned] = useState<number | null>(null);
  const [tableOpen, setTableOpen] = useState(false);
  const tableWrap = useRef<HTMLDivElement>(null);
  const selectedRow = useRef<HTMLTableRowElement>(null);
  const selected = nearest(points, hover === null ? pinned : hover);
  const timestampAt = (clientX: number, bounds: DOMRect) => {
    const fraction = Math.max(0, Math.min(1, ((clientX - bounds.left) / bounds.width * WIDTH - LEFT) / (RIGHT - LEFT)));
    return from + fraction * (to - from);
  };
  useEffect(() => {
    const wrap = tableWrap.current;
    const row = selectedRow.current;
    if (!tableOpen || pinned === null || wrap === null || row === null) return;
    wrap.scrollTop = row.offsetTop - (wrap.clientHeight - row.clientHeight) / 2;
  }, [pinned, tableOpen]);
  return {
    selected, pinned, tableOpen, tableWrap, selectedRow, setTableOpen,
    onPointerMove: (event: React.PointerEvent<SVGSVGElement>) => setHover(timestampAt(event.clientX, event.currentTarget.getBoundingClientRect())),
    onPointerLeave: () => setHover(null),
    onClick: (event: React.MouseEvent<SVGSVGElement>) => {
      const point = nearest(points, timestampAt(event.clientX, event.currentTarget.getBoundingClientRect()));
      if (point === undefined) return;
      setPinned(point.timestamp_epoch_ms);
      setTableOpen(true);
    },
  };
}

export function HistoryChart({ history, device }: { history: ValveHistory; device: string }) {
  const inspection = useHistoryInspection(history.points, history.from_epoch_ms, history.to_epoch_ms);
  const chart = useMemo(() => {
    const temperatures = history.points.flatMap(point => [
      point.freshness === 'fresh' ? point.local_temperature : null,
      point.freshness === 'fresh' ? point.reported_setpoint : null,
      point.target !== null && point.target.kind === 'setpoint' ? point.target.temperature : null,
    ]).filter((value): value is number => value !== null);
    const low = temperatures.length === 0 ? 15 : Math.floor(Math.min(...temperatures) - 1);
    const high = temperatures.length === 0 ? 25 : Math.max(low + 4, Math.ceil(Math.max(...temperatures) + 1));
    const x = (timestamp: number) => LEFT + (timestamp - history.from_epoch_ms) / (history.to_epoch_ms - history.from_epoch_ms) * (RIGHT - LEFT);
    const y = (temperature: number) => BOTTOM - (temperature - low) / (high - low) * (BOTTOM - TOP);
    const actual = chartSegments(history.points, point => point.freshness === 'fresh' ? point.local_temperature : null, x, y, false);
    const reported = chartSegments(history.points, point => point.freshness === 'fresh' ? point.reported_setpoint : null, x, y, true);
    const target = chartSegments(history.points, point => point.target !== null && point.target.kind === 'setpoint' ? point.target.temperature : null, x, y, true);
    const demand = chartSegments(history.points, point => point.freshness === 'fresh' ? point.heating_demand : null, x,
      value => BOTTOM - value / 100 * 24, true);
    const heating = heatingRegions(history).map(region => ({ x: x(region.from), width: x(region.to) - x(region.from) }));
    return { low, high, x, y, actual, reported, target, demand, heating };
  }, [history]);
  const selected = inspection.selected;
  const formatTime = (timestamp: number) => new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const hasReadings = history.points.some(point => point.freshness === 'fresh' && point.local_temperature !== null);
  return <div className="history-chart">
    <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label={`${device}: temperature and setpoint history for the last 24 hours`}
      onPointerMove={inspection.onPointerMove} onPointerLeave={inspection.onPointerLeave} onClick={inspection.onClick}>
      {chart.heating.map((region, index) => <rect className="chart-heating-region" x={region.x} y={TOP} width={region.width} height={BOTTOM - TOP} key={index} />)}
      {[chart.low, (chart.low + chart.high) / 2, chart.high].map(value => <g key={value}>
        <line className="chart-grid" x1={LEFT} y1={chart.y(value)} x2={RIGHT} y2={chart.y(value)} />
        <text className="chart-label" x={LEFT - 7} y={chart.y(value) + 4} textAnchor="end">{value}°</text>
      </g>)}
      <path className="chart-demand" d={chart.demand} />
      <path className="chart-target" d={chart.target} />
      <path className="chart-reported" d={chart.reported} />
      <path className="chart-actual" d={chart.actual} />
      {[-24, -18, -12, -6, 0].map(hours => <text key={hours} className="chart-label" x={chart.x(history.to_epoch_ms + hours * 3600_000)} y={HEIGHT - 5} textAnchor={hours === -24 ? 'start' : hours === 0 ? 'end' : 'middle'}>{hours === 0 ? 'Now' : `${hours}h`}</text>)}
      {selected !== undefined && <line className="chart-cursor" x1={chart.x(selected.timestamp_epoch_ms)} x2={chart.x(selected.timestamp_epoch_ms)} y1={TOP} y2={BOTTOM} />}
    </svg>
    {history.points.length === 0 ? <p className="chart-empty">History starts as the controller records samples.</p>
      : !hasReadings && <p className="chart-empty">No fresh temperature readings in this period.</p>}
    <div className="chart-legend"><span className="legend-actual">Temperature</span><span className="legend-target">Requested</span><span className="legend-reported">Reported setpoint</span><span className="legend-demand">Demand 0–100%</span><span className="legend-heating">Heating active</span></div>
    {selected !== undefined && <div className="chart-inspect">{formatTime(selected.timestamp_epoch_ms)} · {celsius(selected.local_temperature)} · requested {valveTarget(selected.target)} · reported {celsius(selected.reported_setpoint)} · demand {selected.heating_demand === null ? '—' : `${selected.heating_demand}%`} · activity {runningState(selected.running_state)} · {selected.freshness}</div>}
    <details className="history-data" open={inspection.tableOpen} onToggle={event => inspection.setTableOpen(event.currentTarget.open)}><summary>Inspect recorded values</summary>{inspection.tableOpen && <div className="history-table-wrap" ref={inspection.tableWrap}><table>
      <thead><tr><th>Time</th><th>Temperature</th><th>Requested</th><th>Reported setpoint</th><th>Demand</th><th>Activity</th><th>Reading</th></tr></thead>
      <tbody>{[...history.points].reverse().map(point => {
        const isSelected = inspection.pinned === point.timestamp_epoch_ms;
        return <tr key={point.timestamp_epoch_ms} ref={isSelected ? inspection.selectedRow : undefined} aria-current={isSelected ? 'time' : undefined}><td>{formatTime(point.timestamp_epoch_ms)}</td><td>{celsius(point.local_temperature)}</td><td>{valveTarget(point.target)}</td><td>{celsius(point.reported_setpoint)}</td><td>{point.heating_demand === null ? '—' : `${point.heating_demand}%`}</td><td>{runningState(point.running_state)}</td><td>{point.freshness}</td></tr>;
      })}</tbody>
    </table></div>}</details>
  </div>;
}

export function PowerHistoryChart({ history, device }: { history: PlugPowerHistory; device: string }) {
  const inspection = useHistoryInspection(history.points, history.from_epoch_ms, history.to_epoch_ms);
  const chart = useMemo(() => {
    const readings = history.points
      .filter(point => point.freshness === 'fresh')
      .flatMap(point => point.power_watts === null ? [] : [point.power_watts]);
    const high = readings.length === 0 ? 10 : Math.max(10, Math.ceil(Math.max(...readings) * 1.1));
    const x = (timestamp: number) => LEFT + (timestamp - history.from_epoch_ms) / (history.to_epoch_ms - history.from_epoch_ms) * (RIGHT - LEFT);
    const y = (watts: number) => BOTTOM - watts / high * (BOTTOM - TOP);
    const power = chartSegments(history.points, point => point.freshness === 'fresh' ? point.power_watts : null, x, y, false);
    return { high, x, y, power };
  }, [history]);
  const selected = inspection.selected;
  const formatTime = (timestamp: number) => new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return <div className="history-chart power-history-chart">
    <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label={`${device}: power history for the last 24 hours`}
      onPointerMove={inspection.onPointerMove} onPointerLeave={inspection.onPointerLeave} onClick={inspection.onClick}>
      {[0, chart.high / 2, chart.high].map(value => <g key={value}>
        <line className="chart-grid" x1={LEFT} y1={chart.y(value)} x2={RIGHT} y2={chart.y(value)} />
        <text className="chart-label" x={LEFT - 7} y={chart.y(value) + 4} textAnchor="end">{Math.round(value)}W</text>
      </g>)}
      <path className="chart-power" d={chart.power} />
      {[-24, -18, -12, -6, 0].map(hours => <text key={hours} className="chart-label" x={chart.x(history.to_epoch_ms + hours * 3600_000)} y={HEIGHT - 5} textAnchor={hours === -24 ? 'start' : hours === 0 ? 'end' : 'middle'}>{hours === 0 ? 'Now' : `${hours}h`}</text>)}
      {selected !== undefined && <line className="chart-cursor" x1={chart.x(selected.timestamp_epoch_ms)} x2={chart.x(selected.timestamp_epoch_ms)} y1={TOP} y2={BOTTOM} />}
    </svg>
    {history.points.length === 0 ? <p className="chart-empty">History starts as the controller records samples.</p>
      : chart.power === '' && <p className="chart-empty">No fresh power readings in this period.</p>}
    <div className="chart-legend"><span className="legend-power">Power</span></div>
    {selected !== undefined && <div className="chart-inspect">{formatTime(selected.timestamp_epoch_ms)} · {selected.power_watts === null ? '—' : `${selected.power_watts.toFixed(1)} W`} · {selected.freshness}</div>}
    <details className="history-data" open={inspection.tableOpen} onToggle={event => inspection.setTableOpen(event.currentTarget.open)}><summary>Inspect recorded values</summary>{inspection.tableOpen && <div className="history-table-wrap" ref={inspection.tableWrap}><table>
      <thead><tr><th>Time</th><th>Power</th><th>Reading</th></tr></thead>
      <tbody>{[...history.points].reverse().map(point => {
        const isSelected = inspection.pinned === point.timestamp_epoch_ms;
        return <tr key={point.timestamp_epoch_ms} ref={isSelected ? inspection.selectedRow : undefined} aria-current={isSelected ? 'time' : undefined}><td>{formatTime(point.timestamp_epoch_ms)}</td><td>{point.power_watts === null ? '—' : `${point.power_watts.toFixed(1)} W`}</td><td>{point.freshness}</td></tr>;
      })}</tbody>
    </table></div>}</details>
  </div>;
}

function celsius(value: number | null): string { return value === null ? '—' : `${temperature(value)}C`; }
function runningState(value: 'unknown' | 'idle' | 'heat'): string { return value === 'heat' ? 'Heating' : value === 'idle' ? 'Idle' : 'Unknown'; }
