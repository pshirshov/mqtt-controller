import { useRef, useState } from 'react';
import type { ControlCommand, Valve, ValveSchedule } from './protocol';

const DAYS = ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'] as const;
type Day = typeof DAYS[number];
type Range = ValveSchedule['days'][Day][number];

function minutes(time: string): number {
  const parts = time.split(':');
  return Number(parts[0]) * 60 + Number(parts[1]);
}

function timeAt(value: number): string {
  return `${String(Math.floor(value / 60)).padStart(2, '0')}:${String(value % 60).padStart(2, '0')}`;
}

function valid(schedule: ValveSchedule): boolean {
  return DAYS.every(day => {
    const ranges = schedule.days[day];
    return ranges.length > 0 && ranges[0]?.start === '00:00' && ranges.at(-1)?.end === '24:00'
      && ranges.every((range, index) => Number.isFinite(range.temperature) && range.temperature >= 5 && range.temperature <= 30
        && Number.isInteger(minutes(range.start)) && Number.isInteger(minutes(range.end))
        && minutes(range.start) < minutes(range.end)
        && (index === 0 || range.start === ranges[index - 1]?.end));
  });
}

export function ValveScheduleControls({ valve, name, disabled, command }: {
  valve: Valve; name: string; disabled: boolean; command: (command: ControlCommand) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState<ValveSchedule | null>(null);
  const [day, setDay] = useState<Day>('monday');

  function open(): void {
    if (valve.schedule_plan === null) return;
    setDraft(structuredClone(valve.schedule_plan));
    setDay('monday');
    dialog.current?.showModal();
  }

  function update(ranges: Range[]): void {
    if (draft === null) return;
    const normalized = ranges.map((range, index) => ({ ...range, start: index === 0 ? '00:00' : ranges[index - 1]!.end }));
    setDraft({ days: { ...draft.days, [day]: normalized } });
  }

  function split(index: number): void {
    if (draft === null) return;
    const ranges = [...draft.days[day]];
    const range = ranges[index]!;
    const middle = timeAt(Math.floor((minutes(range.start) + minutes(range.end)) / 2));
    ranges.splice(index, 1, { ...range, end: middle }, { ...range, start: middle });
    update(ranges);
  }

  function remove(index: number): void {
    if (draft === null) return;
    const ranges = [...draft.days[day]];
    ranges.splice(index, 1);
    if (index === draft.days[day].length - 1) ranges[ranges.length - 1] = { ...ranges[ranges.length - 1]!, end: '24:00' };
    update(ranges);
  }

  return <>
    <button type="button" className="button schedule-button" disabled={disabled || valve.schedule_plan === null} onClick={open}>
      Edit schedule{valve.schedule_override ? ' · Override active' : ''}
    </button>
    {valve.schedule_override && <button type="button" className="button" aria-label="Restore default schedule" disabled={disabled}
      onClick={() => command({ kind: 'ResetValveSchedule', device: valve.device })}>Restore defaults</button>}
    <dialog ref={dialog} className="schedule-dialog" aria-label={`Schedule for ${name}`}>
      {draft !== null && <form onSubmit={event => {
        event.preventDefault();
        if (!valid(draft)) return;
        command({ kind: 'SetValveSchedule', device: valve.device, schedule: draft });
        dialog.current?.close();
      }}>
        <div className="schedule-dialog-heading"><div><h2>{name} schedule</h2><p>Changes repeat every week and survive controller restarts.</p></div>
          <button type="button" className="text-button" aria-label="Close schedule editor" onClick={() => dialog.current?.close()}>Close</button></div>
        <label className="schedule-day">Day <select value={day} onChange={event => setDay(event.target.value as Day)}>
          {DAYS.map(value => <option key={value} value={value}>{value.charAt(0).toUpperCase() + value.slice(1)}</option>)}
        </select></label>
        <div className="schedule-ranges">{draft.days[day].map((range, index, ranges) => <div className="schedule-range" key={index}>
          <span>{range.start}</span><span>to</span>
          {index === ranges.length - 1 ? <span>24:00</span> : <input type="time" aria-label={`End of period ${index + 1}`} value={range.end}
            onChange={event => update(ranges.map((item, n) => n === index ? { ...item, end: event.target.value } : item))} />}
          <label><input type="number" aria-label={`Temperature for period ${index + 1}`} min="5" max="30" step="0.1" required
            value={range.temperature} onChange={event => update(ranges.map((item, n) => n === index ? { ...item, temperature: Number(event.target.value) } : item))} /> °C</label>
          <button type="button" className="text-button" disabled={minutes(range.end) - minutes(range.start) < 2} onClick={() => split(index)}>Split</button>
          <button type="button" className="text-button" disabled={ranges.length === 1} onClick={() => remove(index)}>Remove</button>
        </div>)}</div>
        {!valid(draft) && <p className="schedule-error" role="status">Each day must cover 00:00–24:00 with increasing times and temperatures from 5–30°C.</p>}
        <div className="schedule-dialog-actions">
          <button type="button" className="button" onClick={() => setDraft({ days: Object.fromEntries(DAYS.map(value => [value, structuredClone(draft.days[day])])) as ValveSchedule['days'] })}>Copy this day to all days</button>
          <button type="submit" className="button primary" disabled={!valid(draft)}>Save schedule</button>
        </div>
      </form>}
    </dialog>
  </>;
}
