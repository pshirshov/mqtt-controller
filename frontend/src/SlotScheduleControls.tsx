import { useRef, useState } from 'react';
import type { SchedulePlan, SlotPlan } from './protocol';
import { label } from './format';
import { validTimeExpression } from './timeExpression';

interface Draft { name: string; from: string; to: string; scenes: string }

function parseScenes(value: string): number[] | null {
  const parts = value.split(',').map(part => part.trim()).filter(part => part !== '');
  const ids = parts.map(Number);
  return ids.every(id => Number.isInteger(id) && id >= 0 && id <= 255) ? ids : null;
}

function slotPlans(drafts: Draft[], available: number[], requireScenes: boolean): SlotPlan[] | null {
  const plans: SlotPlan[] = [];
  for (const draft of drafts) {
    const sceneIds = parseScenes(draft.scenes);
    if (!validTimeExpression(draft.from) || !validTimeExpression(draft.to) || sceneIds === null
      || !sceneIds.every(id => available.includes(id)) || (requireScenes && sceneIds.length === 0)) return null;
    plans.push({ name: draft.name, from: draft.from, to: draft.to, scene_ids: sceneIds });
  }
  return plans;
}

/** Edits slot boundaries and scene order; slot names stay as deployed. */
export function SlotScheduleControls({ title, name, schedule, requireScenes, disabled, save, reset }: {
  title: string; name: string; schedule: SchedulePlan; requireScenes: boolean; disabled: boolean;
  save: (slots: SlotPlan[]) => void; reset: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [drafts, setDrafts] = useState<Draft[] | null>(null);
  const plans = drafts === null ? null : slotPlans(drafts, schedule.available_scene_ids, requireScenes);

  function open(): void {
    setDrafts(schedule.slots.map(slot => ({ name: slot.name, from: slot.from, to: slot.to, scenes: slot.scene_ids.join(', ') })));
    dialog.current?.showModal();
  }

  function change(index: number, field: 'from' | 'to' | 'scenes', value: string): void {
    if (drafts === null) return;
    setDrafts(drafts.map((draft, n) => n === index ? { ...draft, [field]: value } : draft));
  }

  return <div className="slot-schedule-controls">
    <button type="button" className="button schedule-button" disabled={disabled || schedule.slots.length === 0} onClick={open}>
      Edit {title}{schedule.overridden ? ' · Override active' : ''}
    </button>
    {schedule.overridden && <button type="button" className="button" aria-label={`Restore default ${title} for ${name}`} disabled={disabled}
      onClick={reset}>Restore defaults</button>}
    <dialog ref={dialog} className="schedule-dialog" aria-label={`${label(title)} for ${name}`}>
      {drafts !== null && <form onSubmit={event => {
        event.preventDefault();
        if (plans === null) return;
        save(plans);
        dialog.current?.close();
      }}>
        <div className="schedule-dialog-heading"><div><h2>{name} {title}</h2><p>Slots repeat daily and survive controller restarts.</p></div>
          <button type="button" className="text-button" aria-label="Close schedule editor" onClick={() => dialog.current?.close()}>Close</button></div>
        <p className="schedule-hint">Times are HH:MM, sunrise or sunset with an optional ±HH:MM offset, or min(a, b) / max(a, b).
          Scenes are comma-separated ids from {schedule.available_scene_ids.join(', ')}; the first scene is used first.</p>
        <div className="schedule-ranges">{drafts.map((draft, index) => <div className="schedule-range slot-range" key={draft.name}>
          <strong>{label(draft.name)}</strong>
          <input type="text" aria-label={`${label(draft.name)} start`} value={draft.from} onChange={event => change(index, 'from', event.target.value)} />
          <span>to</span>
          <input type="text" aria-label={`${label(draft.name)} end`} value={draft.to} onChange={event => change(index, 'to', event.target.value)} />
          <label>Scenes <input type="text" aria-label={`${label(draft.name)} scenes`} value={draft.scenes} onChange={event => change(index, 'scenes', event.target.value)} /></label>
        </div>)}</div>
        {plans === null && <p className="schedule-error" role="status">Check the time expressions and use only listed scene ids{requireScenes ? ', with at least one scene per slot' : ''}.</p>}
        <p className="schedule-hint">The controller checks that slots cover the whole day without overlaps.</p>
        <div className="schedule-dialog-actions">
          <button type="submit" className="button primary" disabled={plans === null}>Save schedule</button>
        </div>
      </form>}
    </dialog>
  </div>;
}
