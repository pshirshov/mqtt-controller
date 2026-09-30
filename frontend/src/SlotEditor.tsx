import { label } from './format';
import type { SceneOption, SchedulePlan, SlotPlan, SwitchSteps } from './protocol';
import { MINUTES_PER_DAY, fixedMinutes, formatMinutes, nominalMinutes, validTimeExpression } from './timeExpression';

export interface StepDraft { key: number; scene_id: number; lights: string[] }
export interface SlotDraft { key: number; name: string; from: string; to: string; scene_ids: number[]; steps: StepDraft[] }
/** Rooms may reshape their slots; motion targets refer to slot names, so motion rules keep them. */
export type SlotMode = 'room' | 'motion';

const TIME_SUGGESTIONS = ['00:00', '06:00', '08:00', '18:00', '22:00', '23:00', '24:00', 'sunrise', 'sunrise+00:30', 'sunset', 'sunset-00:30', 'sunset+01:00'];
let nextKey = 1;
function key(): number { return nextKey++; }

export function slotDrafts(schedule: SchedulePlan, steps: SwitchSteps): SlotDraft[] {
  return schedule.slots
    .map(slot => ({
      key: key(), name: slot.name, from: slot.from, to: slot.to, scene_ids: [...slot.scene_ids],
      steps: (steps[slot.name] ?? []).map(step => ({ key: key(), scene_id: step.scene_id, lights: [...step.lights] })),
    }))
    .sort((a, b) => nominalMinutes(a.from) - nominalMinutes(b.from));
}

const byName = (a: { name: string }, b: { name: string }) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0;

/** Slots in name order, which is how the controller reports them back. */
export function slotPlans(drafts: SlotDraft[]): SlotPlan[] {
  return drafts.map(draft => ({ name: draft.name.trim(), from: draft.from.trim(), to: draft.to.trim(), scene_ids: draft.scene_ids })).sort(byName);
}

export function switchStepPlans(drafts: SlotDraft[]): SwitchSteps {
  return Object.fromEntries([...drafts].sort(byName).filter(draft => draft.steps.length > 0)
    .map(draft => [draft.name.trim(), draft.steps.map(step => ({ scene_id: step.scene_id, lights: step.lights }))]));
}

/** First problem a controller would reject, or null. Coverage is checked only for fixed times. */
export function slotProblem(drafts: SlotDraft[], mode: SlotMode): string | null {
  if (drafts.length === 0) return 'The schedule needs at least one slot.';
  const names = drafts.map(draft => draft.name.trim());
  if (names.some(name => name === '')) return 'Every slot needs a name.';
  if (new Set(names).size !== names.length) return 'Slot names must be unique.';
  for (const draft of drafts) {
    const name = label(draft.name.trim());
    if (!validTimeExpression(draft.from.trim()) || !validTimeExpression(draft.to.trim())) {
      return `${name}: times are HH:MM, sunrise or sunset with an optional ±HH:MM offset, or min(a, b) / max(a, b).`;
    }
    if (mode === 'motion' && draft.scene_ids.length === 0) return `${name} needs at least one scene.`;
    if (draft.steps.some(step => step.lights.length === 0)) return `${name}: every switch step needs at least one light.`;
  }
  const fixed = drafts.map(draft => [fixedMinutes(draft.from.trim()), fixedMinutes(draft.to.trim())] as const);
  const owners = Array.from({ length: MINUTES_PER_DAY }, () => 0);
  for (const [from, to] of fixed) {
    if (from === null || to === null) return null;
    for (let minute = 0; minute < MINUTES_PER_DAY; minute += 1) {
      if (from <= to ? minute >= from && minute < to : minute >= from || minute < to) owners[minute] = (owners[minute] ?? 0) + 1;
    }
  }
  const overlap = owners.findIndex(count => count > 1);
  if (overlap >= 0) return `${formatMinutes(overlap)} is covered by more than one slot.`;
  const gap = owners.findIndex(count => count === 0);
  if (gap >= 0) {
    let end = gap;
    while (end < MINUTES_PER_DAY && owners[end] === 0) end += 1;
    return `Slots leave ${formatMinutes(gap)}–${formatMinutes(end)} uncovered.`;
  }
  return null;
}

function memberLabel(member: string): string {
  return label(member.replace(/\/\d+$/, ''));
}

function sceneLabel(scenes: SceneOption[], id: number): string {
  const scene = scenes.find(scene => scene.id === id);
  return scene === undefined ? `Scene ${id}` : `${id} · ${label(scene.name)}`;
}

export function SlotList({ drafts, scenes, members, mode, onChange }: {
  drafts: SlotDraft[]; scenes: SceneOption[]; members: string[]; mode: SlotMode; onChange: (drafts: SlotDraft[]) => void;
}) {
  const update = (index: number, patch: Partial<SlotDraft>) => onChange(drafts.map((draft, n) => n === index ? { ...draft, ...patch } : draft));
  const add = () => onChange([...drafts, { key: key(), name: '', from: '', to: '', scene_ids: scenes.length > 0 ? [scenes[0]!.id] : [], steps: [] }]);
  const splitPoint = (draft: SlotDraft): string | null => {
    const [from, to] = [fixedMinutes(draft.from.trim()), fixedMinutes(draft.to.trim())];
    if (from === null || to === null) return null;
    const length = (to - from + MINUTES_PER_DAY) % MINUTES_PER_DAY;
    return length < 2 ? null : formatMinutes((from + Math.floor(length / 2)) % MINUTES_PER_DAY);
  };
  const split = (index: number) => {
    const draft = drafts[index]!;
    const middle = splitPoint(draft);
    if (middle === null) return;
    const base = draft.name.trim() === '' ? 'slot' : draft.name.trim();
    let name = `${base}-2`;
    for (let n = 3; drafts.some(item => item.name.trim() === name); n += 1) name = `${base}-${n}`;
    const second: SlotDraft = { ...draft, key: key(), name, from: middle, scene_ids: [...draft.scene_ids], steps: draft.steps.map(step => ({ ...step, key: key(), lights: [...step.lights] })) };
    onChange(drafts.flatMap((item, n) => n === index ? [{ ...item, to: middle }, second] : [item]));
  };
  return <div className="slot-list">
    <datalist id="time-expressions">{TIME_SUGGESTIONS.map(value => <option key={value} value={value} />)}</datalist>
    {drafts.map((draft, index) => {
      const name = draft.name.trim() === '' ? `slot ${index + 1}` : label(draft.name.trim());
      return <div className="slot-card" key={draft.key} aria-label={`Slot ${name}`}>
        <div className="slot-head">
          {mode === 'room'
            ? <input type="text" aria-label={`Name of slot ${index + 1}`} placeholder="Slot name" value={draft.name} onChange={event => update(index, { name: event.target.value })} />
            : <strong>{name}</strong>}
          {mode === 'room' && <button type="button" className="text-button" disabled={splitPoint(draft) === null} aria-label={`Split slot ${name}`} onClick={() => split(index)}>Split</button>}
          {mode === 'room' && <button type="button" className="text-button" disabled={drafts.length === 1} aria-label={`Remove slot ${name}`}
            onClick={() => onChange(drafts.filter((_, n) => n !== index))}>Remove</button>}
        </div>
        <div className="slot-times">
          <span>From</span><input type="text" list="time-expressions" aria-label={`${name} start`} value={draft.from} onChange={event => update(index, { from: event.target.value })} />
          <span>to</span><input type="text" list="time-expressions" aria-label={`${name} end`} value={draft.to} onChange={event => update(index, { to: event.target.value })} />
        </div>
        <div className="chip-row" role="group" aria-label={`${name} scenes`}>
          {draft.scene_ids.map((id, position) => <span className="chip" key={id}>{position + 1}. {sceneLabel(scenes, id)}
            <button type="button" aria-label={`Remove scene ${id} from ${name}`} onClick={() => update(index, { scene_ids: draft.scene_ids.filter(item => item !== id) })}>×</button></span>)}
          {scenes.some(scene => !draft.scene_ids.includes(scene.id)) && <select className="chip-add" aria-label={`Add scene to ${name}`} value=""
            onChange={event => update(index, { scene_ids: [...draft.scene_ids, Number(event.target.value)] })}>
            <option value="" disabled>{draft.scene_ids.length === 0 ? 'Add scene…' : 'Then…'}</option>
            {scenes.filter(scene => !draft.scene_ids.includes(scene.id)).map(scene => <option key={scene.id} value={scene.id}>{sceneLabel(scenes, scene.id)}</option>)}
          </select>}
        </div>
        {mode === 'room' && <StepList draft={draft} name={name} scenes={scenes} members={members} onChange={steps => update(index, { steps })} />}
      </div>;
    })}
    {mode === 'room' && <button type="button" className="button" onClick={add}>Add slot</button>}
  </div>;
}

/** Ordered switch steps of one slot. Each press applies the next step's scene to its lights and turns the rest off. */
function StepList({ draft, name, scenes, members, onChange }: {
  draft: SlotDraft; name: string; scenes: SceneOption[]; members: string[]; onChange: (steps: StepDraft[]) => void;
}) {
  const update = (index: number, patch: Partial<StepDraft>) => onChange(draft.steps.map((step, n) => n === index ? { ...step, ...patch } : step));
  return <div className="step-list" role="group" aria-label={`${name} switch steps`}>
    <div className="step-heading"><span>Switch steps</span>
      <small>{draft.steps.length === 0 ? 'Each press cycles the whole group' : 'Each press turns on the chosen lights and turns the others off'}</small></div>
    {draft.steps.map((step, index) => <div className="step-row" key={step.key}>
      <span className="step-index">{index + 1}.</span>
      <select aria-label={`Scene of ${name} step ${index + 1}`} value={step.scene_id} onChange={event => update(index, { scene_id: Number(event.target.value) })}>
        {scenes.map(scene => <option key={scene.id} value={scene.id}>{sceneLabel(scenes, scene.id)}</option>)}
      </select>
      {members.map(member => <button type="button" className="light-toggle" key={member} aria-pressed={step.lights.includes(member)}
        aria-label={`${memberLabel(member)} in ${name} step ${index + 1}`}
        onClick={() => update(index, { lights: step.lights.includes(member) ? step.lights.filter(item => item !== member) : members.filter(item => item === member || step.lights.includes(item)) })}>
        {memberLabel(member)}</button>)}
      <button type="button" className="text-button" aria-label={`Remove ${name} step ${index + 1}`} onClick={() => onChange(draft.steps.filter((_, n) => n !== index))}>Remove</button>
    </div>)}
    <button type="button" className="text-button" disabled={members.length === 0 || scenes.length === 0}
      onClick={() => onChange([...draft.steps, { key: key(), scene_id: draft.steps.at(-1)?.scene_id ?? draft.scene_ids[0] ?? scenes[0]!.id, lights: [...members] }])}>Add step</button>
  </div>;
}
