import { useEffect, useState } from 'react';
import type { ControlCommand, Valve } from './protocol';

const BOOST_DURATIONS = [
  { minutes: 30, label: '30m' }, { minutes: 60, label: '1h' }, { minutes: 90, label: '1h 30m' },
  { minutes: 120, label: '2h' }, { minutes: 180, label: '3h' }, { minutes: 240, label: '4h' }, { minutes: 360, label: '6h' },
] as const;
const DEFAULT_BOOST_TARGET = 22;
const MILLIS_PER_MINUTE = 60_000;

export function ValveBoostControls({ device, name, boost, receivedAt, disabled, command }: {
  device: string; name: string; boost: Valve['boost']; receivedAt: number; disabled: boolean;
  command: (command: ControlCommand) => void;
}) {
  const [minutes, setMinutes] = useState<number>(BOOST_DURATIONS[0].minutes);
  const [draft, setDraft] = useState(String(boost === null ? DEFAULT_BOOST_TARGET : boost.temperature));
  const [now, setNow] = useState(Date.now);
  const target = boost === null ? DEFAULT_BOOST_TARGET : boost.temperature;
  const active = boost !== null;
  useEffect(() => { setDraft(String(target)); }, [target, disabled, active]);
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);

  const save = (input: HTMLInputElement) => {
    if (disabled || boost === null) return;
    if (!input.reportValidity()) return;
    const temperature = input.valueAsNumber;
    if (temperature !== boost.temperature) {
      command({ kind: 'SetValveBoostTarget', device, temperature });
    }
  };
  const remainingMinutes = boost === null ? 0
    : Math.ceil(Math.max(0, boost.remaining_ms - Math.max(0, now - receivedAt)) / MILLIS_PER_MINUTE);
  const hours = Math.floor(remainingMinutes / 60);
  const remaining = hours > 0 ? `${hours}h${remainingMinutes % 60 === 0 ? '' : ` ${remainingMinutes % 60}m`}` : `${remainingMinutes}m`;

  return <div className="valve-boost">
    {boost === null ? <div className="boost-controls">
      <select aria-label={`Boost duration for ${name}`} value={minutes} disabled={disabled} onChange={event => setMinutes(Number(event.target.value))}>
        {BOOST_DURATIONS.map(duration => <option key={duration.minutes} value={duration.minutes}>{duration.label}</option>)}
      </select>
      <button type="button" className="button primary" aria-label={`Boost ${name}`} disabled={disabled}
        onClick={() => command({ kind: 'StartValveBoost', device, duration_minutes: minutes, temperature: DEFAULT_BOOST_TARGET })}>Boost</button>
    </div> : <>
      <div className="boost-controls">
        <label>Boost target <span><input type="number" aria-label={`Boost target for ${name}`} value={draft}
          min={5} max={30} step={0.5} required disabled={disabled} onChange={event => setDraft(event.target.value)}
          onBlur={event => save(event.currentTarget)} onKeyDown={event => {
            if (event.key === 'Enter') { event.preventDefault(); save(event.currentTarget); }
            if (event.key === 'Escape') setDraft(String(target));
          }} /> °C</span></label>
        <button type="button" className="button off-button" aria-label={`Cancel boost for ${name}`} disabled={disabled}
          onPointerDown={event => event.preventDefault()}
          onClick={() => command({ kind: 'CancelValveBoost', device })}>Cancel boost</button>
      </div>
      <p className="boost-status" role="status">Boost · {remainingMinutes > 0 ? `${remaining} remaining` : 'Awaiting controller expiry'}{disabled ? ' · Controls unavailable' : ''}</p>
    </>}
    <small>Boost uses the valve’s heat demand. Pump and open-window protection still apply.</small>
  </div>;
}
