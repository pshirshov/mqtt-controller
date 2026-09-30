import { label } from './format';
import type { ControlCommand, TimedAction } from './protocol';
import { validTimeExpression } from './timeExpression';

/** "turn_off → ensuite" as deployed, rendered for people. */
export function describeAction(action: string): string {
  const [effect, target] = action.split(' → ');
  const words = (effect ?? action).replaceAll('_', ' ');
  return target === undefined ? words : `${words} → ${label(target)}`;
}

export type TimedActionDrafts = Record<string, string>;

export function timedActionDrafts(actions: TimedAction[]): TimedActionDrafts {
  return Object.fromEntries(actions.map(action => [action.binding, action.time]));
}

export function timedActionProblem(actions: TimedAction[], drafts: TimedActionDrafts): string | null {
  const invalid = actions.find(action => !validTimeExpression((drafts[action.binding] ?? '').trim()));
  return invalid === undefined ? null
    : `${label(invalid.binding)}: times are HH:MM, sunrise or sunset with an optional ±HH:MM offset, or min(a, b) / max(a, b).`;
}

/** Commands that bring the controller to the drafted times; empty when nothing changed. */
export function timedActionCommands(actions: TimedAction[], drafts: TimedActionDrafts): [string, ControlCommand][] {
  return actions
    .filter(action => (drafts[action.binding] ?? '').trim() !== action.time)
    .map(action => [action.binding, { kind: 'SetTimedActionTime', binding: action.binding, time: (drafts[action.binding] ?? '').trim() }]);
}

export function TimedActionRows({ actions, drafts, onChange }: { actions: TimedAction[]; drafts: TimedActionDrafts; onChange: (drafts: TimedActionDrafts) => void }) {
  return <div className="dialog-rows">{actions.map(action => {
    const name = label(action.binding);
    const value = drafts[action.binding] ?? '';
    return <div className="dialog-row" key={action.binding}>
      <span><strong>{name}</strong><small>{describeAction(action.action)}{action.overridden ? ' · Override active' : ''}</small></span>
      <input type="text" list="time-expressions" aria-label={`Time for ${name}`} aria-invalid={!validTimeExpression(value.trim())}
        value={value} onChange={event => onChange({ ...drafts, [action.binding]: event.target.value })} />
    </div>;
  })}</div>;
}
