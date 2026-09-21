import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { PlugPowerHistory, ValveHistory } from './protocol';
import { temperature, valveTarget } from './format';
import { MAX_SAMPLE_GAP_MS, type RelayHistory } from './energy';

const WIDTH = 480;
const HEIGHT = 190;
const LEFT = 60;
const TOP = 12;

interface Timestamped { timestamp_epoch_ms: number }
interface HeatingRegion { from: number; to: number }
interface PointerPosition { x: number; y: number }

function useChartLayout(compact: boolean) {
  const container = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(WIDTH);
  useEffect(() => {
    const element = container.current;
    if (element === null) return;
    const observer = new ResizeObserver(entries => {
      const entry = entries[0];
      if (entry !== undefined && entry.contentRect.width > 0) setWidth(entry.contentRect.width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const height = compact ? 140 : HEIGHT;
  return { container, width, height, right: width - 10, bottom: height - 24 };
}

function ChartTooltip({ position, children }: { position: PointerPosition | null; children: React.ReactNode }) {
  if (position === null) return null;
  return createPortal(<div role="tooltip" className="chart-tooltip" style={{
    left: Math.max(8, Math.min(position.x + 14, window.innerWidth - 278)),
    top: Math.max(8, Math.min(position.y + 14, window.innerHeight - 250)),
  }}>{children}</div>, document.body);
}

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

function useHistoryInspection<T extends Timestamped>(points: T[], from: number, to: number, width: number) {
  const [hover, setHover] = useState<number | null>(null);
  const [pinned, setPinned] = useState<number | null>(null);
  const [tableOpen, setTableOpen] = useState(false);
  const [pointer, setPointer] = useState<PointerPosition | null>(null);
  const tableWrap = useRef<HTMLDivElement>(null);
  const selectedRow = useRef<HTMLTableRowElement>(null);
  const selected = nearest(points, hover === null ? pinned : hover);
  const timestampAt = (clientX: number, bounds: DOMRect) => {
    const fraction = Math.max(0, Math.min(1, ((clientX - bounds.left) / bounds.width * width - LEFT) / (width - 10 - LEFT)));
    return from + fraction * (to - from);
  };
  useEffect(() => {
    const wrap = tableWrap.current;
    const row = selectedRow.current;
    if (!tableOpen || pinned === null || wrap === null || row === null) return;
    wrap.scrollTop = row.offsetTop - (wrap.clientHeight - row.clientHeight) / 2;
  }, [pinned, tableOpen]);
  return {
    selected, pinned, tableOpen, tableWrap, selectedRow, setTableOpen, pointer,
    onPointerMove: (event: React.PointerEvent<SVGSVGElement>) => {
      setPointer({ x: event.clientX, y: event.clientY });
      setHover(timestampAt(event.clientX, event.currentTarget.getBoundingClientRect()));
    },
    onPointerLeave: () => { setHover(null); setPointer(null); },
    onClick: (event: React.MouseEvent<SVGSVGElement>) => {
      const point = nearest(points, timestampAt(event.clientX, event.currentTarget.getBoundingClientRect()));
      if (point === undefined) return;
      setPointer({ x: event.clientX, y: event.clientY });
      setPinned(point.timestamp_epoch_ms);
      setTableOpen(true);
    },
  };
}

export function HistoryChart({ history, device, compact }: { history: ValveHistory; device: string; compact: boolean }) {
  const { container, width, height, right, bottom } = useChartLayout(compact);
  const inspection = useHistoryInspection(history.points, history.from_epoch_ms, history.to_epoch_ms, width);
  const chart = useMemo(() => {
    const temperatures = history.points.flatMap(point => [
      point.freshness === 'fresh' ? point.local_temperature : null,
      point.freshness === 'fresh' ? point.reported_setpoint : null,
      point.target !== null && point.target.kind === 'setpoint' ? point.target.temperature : null,
    ]).filter((value): value is number => value !== null);
    const low = temperatures.length === 0 ? 15 : Math.floor(Math.min(...temperatures) - 1);
    const high = temperatures.length === 0 ? 25 : Math.max(low + 4, Math.ceil(Math.max(...temperatures) + 1));
    const x = (timestamp: number) => LEFT + (timestamp - history.from_epoch_ms) / (history.to_epoch_ms - history.from_epoch_ms) * (right - LEFT);
    const y = (temperature: number) => bottom - (temperature - low) / (high - low) * (bottom - TOP);
    const actual = chartSegments(history.points, point => point.freshness === 'fresh' ? point.local_temperature : null, x, y, false);
    const reported = chartSegments(history.points, point => point.freshness === 'fresh' ? point.reported_setpoint : null, x, y, true);
    const target = chartSegments(history.points, point => point.target !== null && point.target.kind === 'setpoint' ? point.target.temperature : null, x, y, true);
    const demand = chartSegments(history.points, point => point.freshness === 'fresh' ? point.heating_demand : null, x,
      value => bottom - value / 100 * 24, true);
    const heating = heatingRegions(history).map(region => ({ x: x(region.from), width: x(region.to) - x(region.from) }));
    return { low, high, x, y, actual, reported, target, demand, heating };
  }, [history, right, bottom]);
  const selected = inspection.selected;
  const formatTime = (timestamp: number) => new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const hasReadings = history.points.some(point => point.freshness === 'fresh' && point.local_temperature !== null);
  return <div className="history-chart" ref={container}>
    <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${device}: temperature and setpoint history for the last 24 hours`}
      onPointerMove={inspection.onPointerMove} onPointerLeave={inspection.onPointerLeave} onClick={inspection.onClick}>
      {chart.heating.map((region, index) => <rect className="chart-heating-region" x={region.x} y={TOP} width={region.width} height={bottom - TOP} key={index} />)}
      {[chart.low, (chart.low + chart.high) / 2, chart.high].map(value => <g key={value}>
        <line className="chart-grid" x1={LEFT} y1={chart.y(value)} x2={right} y2={chart.y(value)} />
        <text className="chart-label" x={LEFT - 7} y={chart.y(value) + 4} textAnchor="end">{value}°</text>
      </g>)}
      <path className="chart-demand" d={chart.demand} />
      <path className="chart-target" d={chart.target} />
      <path className="chart-reported" d={chart.reported} />
      <path className="chart-actual" d={chart.actual} />
      {[-24, -18, -12, -6, 0].map(hours => <text key={hours} className="chart-label" x={chart.x(history.to_epoch_ms + hours * 3600_000)} y={height - 5} textAnchor={hours === -24 ? 'start' : hours === 0 ? 'end' : 'middle'}>{hours === 0 ? 'Now' : `${hours}h`}</text>)}
      {selected !== undefined && <line className="chart-cursor" x1={chart.x(selected.timestamp_epoch_ms)} x2={chart.x(selected.timestamp_epoch_ms)} y1={TOP} y2={bottom} />}
    </svg>
    {history.points.length === 0 ? <p className="chart-empty">History starts as the controller records samples.</p>
      : !hasReadings && <p className="chart-empty">No fresh temperature readings in this period.</p>}
    <div className="chart-legend"><span className="legend-actual">Temperature</span><span className="legend-target">Requested</span><span className="legend-reported">Reported setpoint</span><span className="legend-demand">Demand 0–100%</span><span className="legend-heating">Heating active</span></div>
    {selected !== undefined && <ChartTooltip position={inspection.pointer}><strong>{device} · {formatTime(selected.timestamp_epoch_ms)}</strong><dl>
      <dt>Temperature</dt><dd>{celsius(selected.local_temperature)}</dd><dt>Requested</dt><dd>{valveTarget(selected.target)}</dd>
      <dt>Reported setpoint</dt><dd>{celsius(selected.reported_setpoint)}</dd><dt>Demand</dt><dd>{selected.heating_demand === null ? '—' : `${selected.heating_demand}%`}</dd>
      <dt>Activity</dt><dd>{runningState(selected.running_state)}</dd><dt>Reading</dt><dd>{selected.freshness}</dd>
    </dl></ChartTooltip>}
    <details className="history-data" open={inspection.tableOpen} onToggle={event => inspection.setTableOpen(event.currentTarget.open)}><summary>Inspect recorded values</summary>{inspection.tableOpen && <div className="history-table-wrap" ref={inspection.tableWrap}><table>
      <thead><tr><th>Time</th><th>Temperature</th><th>Requested</th><th>Reported setpoint</th><th>Demand</th><th>Activity</th><th>Reading</th></tr></thead>
      <tbody>{[...history.points].reverse().map(point => {
        const isSelected = inspection.pinned === point.timestamp_epoch_ms;
        return <tr key={point.timestamp_epoch_ms} ref={isSelected ? inspection.selectedRow : undefined} aria-current={isSelected ? 'time' : undefined}><td>{formatTime(point.timestamp_epoch_ms)}</td><td>{celsius(point.local_temperature)}</td><td>{valveTarget(point.target)}</td><td>{celsius(point.reported_setpoint)}</td><td>{point.heating_demand === null ? '—' : `${point.heating_demand}%`}</td><td>{runningState(point.running_state)}</td><td>{point.freshness}</td></tr>;
      })}</tbody>
    </table></div>}</details>
  </div>;
}

type PowerHistory = Pick<PlugPowerHistory, 'points' | 'from_epoch_ms' | 'to_epoch_ms'>;
export function PowerHistoryChart({ history, device, compact }: { history: PowerHistory; device: string; compact: boolean }) {
  const { container, width, height, right, bottom } = useChartLayout(compact);
  const inspection = useHistoryInspection(history.points, history.from_epoch_ms, history.to_epoch_ms, width);
  const chart = useMemo(() => {
    const readings = history.points
      .filter(point => point.freshness === 'fresh')
      .flatMap(point => point.power_watts === null ? [] : [point.power_watts]);
    const high = readings.length === 0 ? 10 : Math.max(10, Math.ceil(Math.max(...readings) * 1.1));
    const x = (timestamp: number) => LEFT + (timestamp - history.from_epoch_ms) / (history.to_epoch_ms - history.from_epoch_ms) * (right - LEFT);
    const y = (watts: number) => bottom - watts / high * (bottom - TOP);
    const power = chartSegments(history.points, point => point.freshness === 'fresh' ? point.power_watts : null, x, y, false);
    return { high, x, y, power };
  }, [history, right, bottom]);
  const selected = inspection.selected;
  const formatTime = (timestamp: number) => new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return <div className="history-chart power-history-chart" ref={container}>
    <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${device}: power history for the last 24 hours`}
      onPointerMove={inspection.onPointerMove} onPointerLeave={inspection.onPointerLeave} onClick={inspection.onClick}>
      {[0, chart.high / 2, chart.high].map(value => <g key={value}>
        <line className="chart-grid" x1={LEFT} y1={chart.y(value)} x2={right} y2={chart.y(value)} />
        <text className="chart-label" x={LEFT - 7} y={chart.y(value) + 4} textAnchor="end">{value >= 1000 ? `${(value / 1000).toFixed(1)}kW` : `${Math.round(value)}W`}</text>
      </g>)}
      <path className="chart-power" d={chart.power} />
      {[-24, -18, -12, -6, 0].map(hours => <text key={hours} className="chart-label" x={chart.x(history.to_epoch_ms + hours * 3600_000)} y={height - 5} textAnchor={hours === -24 ? 'start' : hours === 0 ? 'end' : 'middle'}>{hours === 0 ? 'Now' : `${hours}h`}</text>)}
      {selected !== undefined && <line className="chart-cursor" x1={chart.x(selected.timestamp_epoch_ms)} x2={chart.x(selected.timestamp_epoch_ms)} y1={TOP} y2={bottom} />}
    </svg>
    {history.points.length === 0 ? <p className="chart-empty">History starts as the controller records samples.</p>
      : chart.power === '' && <p className="chart-empty">No fresh power readings in this period.</p>}
    <div className="chart-legend"><span className="legend-power">Power</span></div>
    {selected !== undefined && <ChartTooltip position={inspection.pointer}><strong>{device} · {formatTime(selected.timestamp_epoch_ms)}</strong><dl>
      <dt>Power</dt><dd>{selected.power_watts === null ? '—' : `${selected.power_watts.toFixed(1)} W`}</dd><dt>Reading</dt><dd>{selected.freshness}</dd>
    </dl></ChartTooltip>}
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

export function RelayHistoryChart({ history, device }: { history: RelayHistory; device: string }) {
  const { container, width, height, right, bottom } = useChartLayout(true);
  const inspection = useHistoryInspection(history.points, history.from_epoch_ms, history.to_epoch_ms, width);
  const x = (timestamp: number) => LEFT + (timestamp - history.from_epoch_ms) / (history.to_epoch_ms - history.from_epoch_ms) * (right - LEFT);
  const y = (value: number) => value === 0 ? bottom : TOP + 12;
  const path = chartSegments(history.points, point => point.freshness !== 'fresh' || point.on === null ? null : Number(point.on), x, y, true);
  const selected = inspection.selected;
  return <div className="history-chart relay-history-chart" ref={container}>
    <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${device}: relay history for the last 24 hours`}
      onPointerMove={inspection.onPointerMove} onPointerLeave={inspection.onPointerLeave} onClick={inspection.onClick}>
      {history.active.map(interval => <rect key={interval.from} className="chart-heating-region" x={x(interval.from)} width={x(interval.to) - x(interval.from)} y={TOP} height={bottom - TOP} />)}
      {[0, 1].map(value => <g key={value}><line className="chart-grid" x1={LEFT} x2={right} y1={y(value)} y2={y(value)} />
        <text className="chart-label" x={LEFT - 7} y={y(value) + 4} textAnchor="end">{value === 0 ? 'Off' : 'On'}</text></g>)}
      <path className="chart-power" d={path} />
      {[-24, -18, -12, -6, 0].map(hours => <text key={hours} className="chart-label" x={x(history.to_epoch_ms + hours * 3600_000)} y={height - 5} textAnchor={hours === -24 ? 'start' : hours === 0 ? 'end' : 'middle'}>{hours === 0 ? 'Now' : `${hours}h`}</text>)}
      {selected !== undefined && <line className="chart-cursor" x1={x(selected.timestamp_epoch_ms)} x2={x(selected.timestamp_epoch_ms)} y1={TOP} y2={bottom} />}
    </svg>
    {history.observed_ms === 0 && <p className="chart-empty">No measured relay intervals in this period.</p>}
    {selected !== undefined && <ChartTooltip position={inspection.pointer}><strong>{device} · {new Date(selected.timestamp_epoch_ms).toLocaleTimeString()}</strong><dl>
      <dt>Reported relay</dt><dd>{selected.on === null ? 'Unknown' : selected.on ? 'On' : 'Off'}</dd><dt>Reading</dt><dd>{selected.freshness}</dd>
    </dl></ChartTooltip>}
    <details className="history-data" open={inspection.tableOpen} onToggle={event => inspection.setTableOpen(event.currentTarget.open)}><summary>Inspect recorded values</summary>
      {inspection.tableOpen && <div className="history-table-wrap" ref={inspection.tableWrap}><table><thead><tr><th>Time</th><th>Relay</th><th>Reading</th></tr></thead>
        <tbody>{[...history.points].reverse().map(point => <tr key={point.timestamp_epoch_ms} ref={inspection.pinned === point.timestamp_epoch_ms ? inspection.selectedRow : undefined} aria-current={inspection.pinned === point.timestamp_epoch_ms ? 'time' : undefined}>
          <td>{new Date(point.timestamp_epoch_ms).toLocaleTimeString()}</td><td>{point.on === null ? 'Unknown' : point.on ? 'On' : 'Off'}</td><td>{point.freshness}</td>
        </tr>)}</tbody></table></div>}
    </details>
  </div>;
}
