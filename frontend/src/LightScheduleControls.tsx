import { useRef, useState } from 'react';
import { TimedActionRows, timedActionCommands, timedActionDrafts, timedActionProblem, type TimedActionDrafts } from './AutomationRows';
import { DialogSection, ScheduleButton, ScheduleDialog } from './ScheduleDialog';
import { SlotList, slotDrafts, slotPlans, slotProblem, switchStepPlans, type SlotDraft } from './SlotEditor';
import { label } from './format';
import type { ControlCommand, Room } from './protocol';

export type Send = (key: string, command: ControlCommand) => void;

interface RoomDraft { slots: SlotDraft[]; times: TimedActionDrafts }

/** Slots, switch steps and timed actions of one light group in a single editor. */
export function LightScheduleControls({ room, commandKey, disabled, send }: { room: Room; commandKey: string; disabled: boolean; send: Send }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState<RoomDraft | null>(null);
  const name = label(room.name);
  const overridden = room.schedule.overridden || room.timed_actions.some(action => action.overridden);
  const scheduleChanged = draft !== null && (JSON.stringify(slotPlans(draft.slots)) !== JSON.stringify(room.schedule.slots)
    || JSON.stringify(switchStepPlans(draft.slots)) !== JSON.stringify(room.switch_steps));
  const timeCommands = draft === null ? [] : timedActionCommands(room.timed_actions, draft.times);
  const problem = draft === null ? null : slotProblem(draft.slots, 'room') ?? timedActionProblem(room.timed_actions, draft.times);

  return <div className="schedule-controls">
    <ScheduleButton label="Edit schedule" overridden={overridden} disabled={disabled || room.schedule.slots.length === 0}
      onClick={() => { setDraft({ slots: slotDrafts(room.schedule, room.switch_steps), times: timedActionDrafts(room.timed_actions) }); dialog.current?.showModal(); }} />
    <ScheduleDialog dialogRef={dialog} label={`Schedule for ${name}`} title={`${name} schedule`}
      subtitle="Slots repeat daily; changes are saved on the controller and survive restarts." error={problem}
      canSave={draft !== null && problem === null && (scheduleChanged || timeCommands.length > 0)} onClose={() => setDraft(null)}
      onSave={() => {
        if (draft === null) return;
        if (scheduleChanged) send(commandKey, { kind: 'SetRoomSchedule', room: room.name, slots: slotPlans(draft.slots), switch_steps: switchStepPlans(draft.slots) });
        for (const [binding, command] of timeCommands) send(`${commandKey}/timed/${binding}`, command);
        dialog.current?.close();
      }}
      onRestore={!overridden ? null : () => {
        if (room.schedule.overridden) send(commandKey, { kind: 'ResetRoomSchedule', room: room.name });
        for (const action of room.timed_actions.filter(action => action.overridden)) {
          send(`${commandKey}/timed/${action.binding}`, { kind: 'ResetTimedActionTime', binding: action.binding });
        }
        dialog.current?.close();
      }}>
      {draft !== null && <>
        <DialogSection title="Time slots" hint="Slots must cover the whole day without overlaps. The first scene of a slot is used first.">
          <SlotList drafts={draft.slots} scenes={room.schedule.available_scenes} members={room.members} mode="room"
            onChange={slots => setDraft({ ...draft, slots })} />
        </DialogSection>
        {room.timed_actions.length > 0 && <DialogSection title="Timed actions" hint="Daily actions on this group. 24:00 never fires.">
          <TimedActionRows actions={room.timed_actions} drafts={draft.times} onChange={times => setDraft({ ...draft, times })} />
        </DialogSection>}
      </>}
    </ScheduleDialog>
  </div>;
}

/** Slot boundaries and scenes of one motion rule. Slot names stay as deployed because targets refer to them. */
export function MotionScheduleControls({ rule, commandKey, disabled, send }: { rule: Room['motion_rules'][number]; commandKey: string; disabled: boolean; send: Send }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [draft, setDraft] = useState<SlotDraft[] | null>(null);
  const name = label(rule.name);
  const changed = draft !== null && JSON.stringify(slotPlans(draft)) !== JSON.stringify(rule.schedule.slots);
  const problem = draft === null ? null : slotProblem(draft, 'motion');
  return <div className="schedule-controls">
    <ScheduleButton label="Edit motion schedule" overridden={rule.schedule.overridden} disabled={disabled || rule.schedule.slots.length === 0}
      onClick={() => { setDraft(slotDrafts(rule.schedule, {})); dialog.current?.showModal(); }} />
    <ScheduleDialog dialogRef={dialog} label={`Motion schedule for ${name}`} title={`${name} schedule`}
      subtitle="Which scenes motion recalls at each time of day. A running motion session keeps its slot." error={problem}
      canSave={changed && problem === null} onClose={() => setDraft(null)}
      onSave={() => { if (draft !== null) send(commandKey, { kind: 'SetMotionSchedule', rule: rule.name, slots: slotPlans(draft) }); dialog.current?.close(); }}
      onRestore={!rule.schedule.overridden ? null : () => { send(commandKey, { kind: 'ResetMotionSchedule', rule: rule.name }); dialog.current?.close(); }}>
      {draft !== null && <DialogSection title="Time slots" hint="Every slot needs at least one scene; the first one is recalled on motion.">
        <SlotList drafts={draft} scenes={rule.schedule.available_scenes} members={[]} mode="motion" onChange={setDraft} />
      </DialogSection>}
    </ScheduleDialog>
  </div>;
}
