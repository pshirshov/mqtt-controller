import { describe, expect, it } from 'vitest';
import { slotProblem, type SlotDraft } from './SlotEditor';
import { dayProblem } from './ValveScheduleControls';
import { fixedMinutes, nominalMinutes } from './timeExpression';

function slot(name: string, from: string, to: string, scene_ids: number[] = [1], lights: string[][] = []): SlotDraft {
  return { key: 0, name, from, to, scene_ids, steps: lights.map(step => ({ key: 0, scene_id: 1, lights: step })) };
}

describe('slotProblem', () => {
  it('accepts a wrapping schedule that tiles the day', () => {
    expect(slotProblem([slot('day', '06:00', '23:00'), slot('night', '23:00', '06:00')], 'room')).toBeNull();
  });
  it('reports gaps and overlaps of fixed-time slots', () => {
    expect(slotProblem([slot('day', '06:00', '22:00'), slot('night', '23:00', '06:00')], 'room')).toBe('Slots leave 22:00–23:00 uncovered.');
    expect(slotProblem([slot('day', '06:00', '23:30'), slot('night', '23:00', '06:00')], 'room')).toBe('23:00 is covered by more than one slot.');
  });
  it('leaves coverage of sun-relative slots to the controller', () => {
    expect(slotProblem([slot('day', 'sunrise', 'sunset-01:00'), slot('night', 'sunset', 'sunrise')], 'room')).toBeNull();
  });
  it('rejects blank or duplicate names, bad times, empty motion scenes and empty steps', () => {
    expect(slotProblem([slot(' ', '00:00', '24:00')], 'room')).toBe('Every slot needs a name.');
    expect(slotProblem([slot('a', '00:00', '12:00'), slot('a', '12:00', '24:00')], 'room')).toBe('Slot names must be unique.');
    expect(slotProblem([slot('day', '6am', '24:00')], 'room')).toContain('Day: times are HH:MM');
    expect(slotProblem([slot('day', '00:00', '24:00', [])], 'motion')).toBe('Day needs at least one scene.');
    expect(slotProblem([slot('day', '00:00', '24:00', [])], 'room')).toBeNull();
    expect(slotProblem([slot('day', '00:00', '24:00', [1], [[]])], 'room')).toBe('Day: every switch step needs at least one light.');
    expect(slotProblem([], 'room')).toBe('The schedule needs at least one slot.');
  });
});

describe('dayProblem', () => {
  it('requires contiguous periods from 00:00 to 24:00 within 5–30°C', () => {
    expect(dayProblem([{ start: '00:00', end: '07:00', temperature: 18 }, { start: '07:00', end: '24:00', temperature: 21 }])).toBeNull();
    expect(dayProblem([{ start: '00:00', end: '07:00', temperature: 18 }, { start: '07:30', end: '24:00', temperature: 21 }])).toBe('Periods must follow each other without gaps.');
    expect(dayProblem([{ start: '00:00', end: '00:00', temperature: 18 }, { start: '00:00', end: '24:00', temperature: 21 }])).toBe('Each period must start before the next one.');
    expect(dayProblem([{ start: '00:00', end: '24:00', temperature: 31 }])).toBe('Targets must be 5–30°C.');
    expect(dayProblem([{ start: '01:00', end: '24:00', temperature: 20 }])).toBe('Periods must run from 00:00 to 24:00.');
  });
});

describe('time expressions', () => {
  it('parses fixed times and orders sun-relative ones nominally', () => {
    expect(fixedMinutes('24:00')).toBe(1440);
    expect(fixedMinutes('06:75')).toBeNull();
    expect(fixedMinutes('sunset')).toBeNull();
    expect(nominalMinutes('sunset-00:30')).toBe(17 * 60 + 30);
    expect(nominalMinutes('max(sunset, 20:00)')).toBe(20 * 60);
    expect(nominalMinutes('min(sunrise+01:00, 06:30)')).toBe(6 * 60 + 30);
  });
});
