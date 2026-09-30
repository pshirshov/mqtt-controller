import { useRef, useState } from 'react';
import type { Send } from './LightScheduleControls';
import { ScheduleButton, ScheduleDialog } from './ScheduleDialog';
import type { Valve, ValveSchedule } from './protocol';
import { fixedMinutes, formatMinutes } from './timeExpression';

const DAYS = ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'] as const;
type Day = typeof DAYS[number];
type Range = ValveSchedule['days'][Day][number];
const WEEKDAYS: readonly Day[] = DAYS.slice(0, 5);
const WEEKEND: readonly Day[] = DAYS.slice(5);
const MIN_TEMPERATURE = 5;
const MAX_TEMPERATURE = 30;
const DAY_START = '00:00';
const DAY_END = '24:00';

function minutes(time: string): number {
  return fixedMinutes(time) ?? Number.NaN;
}

/** The controller's own rule: each day tiles 00:00–24:00 with increasing times and 5–30°C targets. */
export function dayProblem(ranges: Range[]): string | null {
  if (ranges.length === 0 || ranges[0]?.start !== DAY_START || ranges.at(-1)?.end !== DAY_END) return 'Periods must run from 00:00 to 24:00.';
  for (const [index, range] of ranges.entries()) {
    if (!Number.isFinite(range.temperature) || range.temperature < MIN_TEMPERATURE || range.temperature > MAX_TEMPERATURE) return `Targets must be ${MIN_TEMPERATURE}–${MAX_TEMPERATURE}°C.`;
    if (!(minutes(range.start) < minutes(range.end))) return 'Each period must start before the next one.';
    if (index > 0 && range.start !== ranges[index - 1]?.end) return 'Periods must follow each other without gaps.';
  }
  return null;
}

function scheduleProblem(schedule: ValveSchedule): string | null {
  for (const day of DAYS) {
    const problem = dayProblem(schedule.days[day]);
    if (problem !== null) return `${capitalize(day)}: ${problem}`;
  }
  return null;
}

function capitalize(day: Day): string {
  return day.charAt(0).toUpperCase() + day.slice(1);
}

export function ValveScheduleControls({ valve, name, commandKey, disabled, send }: { valve: Valve; name: string; commandKey: string; disabled: boolean; send: Send }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState<ValveSchedule | null>(null);
  const [day, setDay] = useState<Day>('monday');
  const problem = draft === null ? null : scheduleProblem(draft);
  const changed = draft !== null && JSON.stringify(draft) !== JSON.stringify(valve.schedule_plan);
  const ranges = draft === null ? [] : draft.days[day];

  function update(next: Range[]): void {
    if (draft === null) return;
    setDraft({ days: { ...draft.days, [day]: next } });
  }
  function setStart(index: number, start: string): void {
    update(ranges.map((range, n) => n === index ? { ...range, start } : n === index - 1 ? { ...range, end: start } : range));
  }
  function addPeriod(): void {
    const last = ranges.at(-1);
    if (last === undefined) return;
    const middle = formatMinutes(Math.floor((minutes(last.start) + minutes(last.end)) / 2));
    update([...ranges.slice(0, -1), { ...last, end: middle }, { start: middle, end: last.end, temperature: last.temperature }]);
  }
  function remove(index: number): void {
    const removed = ranges[index];
    if (removed === undefined) return;
    update(ranges.filter((_, n) => n !== index).map((range, n) => n === index - 1 ? { ...range, end: removed.end } : range));
  }
  function copyTo(days: readonly Day[]): void {
    if (draft === null) return;
    setDraft({ days: { ...draft.days, ...Object.fromEntries(days.map(target => [target, structuredClone(ranges)])) } });
  }
  const last = ranges.at(-1);
  const canAdd = last !== undefined && minutes(last.end) - minutes(last.start) >= 2;

  return <>
    <ScheduleButton label="Edit schedule" overridden={valve.schedule_override} disabled={disabled || valve.schedule_plan === null}
      onClick={() => { if (valve.schedule_plan === null) return; setDraft(structuredClone(valve.schedule_plan)); setDay('monday'); dialog.current?.showModal(); }} />
    <ScheduleDialog dialogRef={dialog} label={`Schedule for ${name}`} title={`${name} schedule`}
      subtitle="Target temperatures by time of day, repeated every week. Saved on the controller; an active Boost still wins until it ends." error={problem}
      canSave={changed && problem === null} onClose={() => setDraft(null)}
      onSave={() => { if (draft !== null) send(commandKey, { kind: 'SetValveSchedule', device: valve.device, schedule: draft }); dialog.current?.close(); }}
      onRestore={!valve.schedule_override ? null : () => { send(commandKey, { kind: 'ResetValveSchedule', device: valve.device }); dialog.current?.close(); }}>
      {draft !== null && <>
        <div className="day-tabs" role="tablist" aria-label="Day of week">
          {DAYS.map(value => <button type="button" role="tab" key={value} aria-selected={value === day} aria-label={capitalize(value)} onClick={() => setDay(value)}>
            {capitalize(value).slice(0, 3)}</button>)}
        </div>
        <div className="period-list" role="group" aria-label={`${capitalize(day)} periods`}>
          {ranges.map((range, index) => <div className="period-row" key={index}>
            {index === 0 ? <span className="period-start">{DAY_START}</span>
              : <input className="period-start" type="time" aria-label={`Start of period ${index + 1}`} value={range.start} onChange={event => setStart(index, event.target.value)} />}
            <label className="unit-field"><input type="number" aria-label={`Temperature for period ${index + 1}`} min={MIN_TEMPERATURE} max={MAX_TEMPERATURE} step="0.1" required
              value={range.temperature} onChange={event => update(ranges.map((item, n) => n === index ? { ...item, temperature: Number(event.target.value) } : item))} /><span>°C</span></label>
            <span className="period-until">until {index === ranges.length - 1 ? DAY_END : range.end}</span>
            <button type="button" className="text-button" disabled={index === 0} aria-label={`Remove period ${index + 1}`} onClick={() => remove(index)}>Remove</button>
          </div>)}
          <button type="button" className="text-button" disabled={!canAdd} onClick={addPeriod}>Add period</button>
        </div>
        <div className="copy-row"><span>Copy {capitalize(day)} to</span>
          <button type="button" className="text-button" onClick={() => copyTo(DAYS)}>all days</button>
          <button type="button" className="text-button" onClick={() => copyTo(WEEKDAYS)}>weekdays</button>
          <button type="button" className="text-button" onClick={() => copyTo(WEEKEND)}>weekend</button>
        </div>
      </>}
    </ScheduleDialog>
  </>;
}
