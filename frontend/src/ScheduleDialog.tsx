import type { ReactNode, RefObject } from 'react';

/** Modal chrome shared by every schedule editor: heading, sections, one error line, restore and save. */
export function ScheduleDialog({ dialogRef, label, title, subtitle, error, canSave, onSave, onRestore, onClose, children }: {
  dialogRef: RefObject<HTMLDialogElement | null>; label: string; title: string; subtitle: string;
  error: string | null; canSave: boolean; onSave: () => void; onRestore: (() => void) | null; onClose: () => void; children: ReactNode;
}) {
  return <dialog ref={dialogRef} className="schedule-dialog" aria-label={label} onClose={onClose}>
    <form onSubmit={event => { event.preventDefault(); if (canSave) onSave(); }}>
      <div className="schedule-dialog-heading"><div><h2>{title}</h2><p>{subtitle}</p></div>
        <button type="button" className="text-button" aria-label="Close schedule editor" onClick={() => dialogRef.current?.close()}>Close</button></div>
      {children}
      {error !== null && <p className="schedule-error" role="status">{error}</p>}
      <div className="schedule-dialog-actions">
        {onRestore !== null ? <button type="button" className="button off-button" onClick={onRestore}>Restore defaults</button> : <span />}
        <button type="submit" className="button primary" disabled={!canSave}>Save schedule</button>
      </div>
    </form>
  </dialog>;
}

export function DialogSection({ title, hint, children }: { title: string; hint: string | null; children: ReactNode }) {
  return <section className="dialog-section">
    <div className="dialog-section-heading"><h3>{title}</h3>{hint !== null && <p>{hint}</p>}</div>
    {children}
  </section>;
}

export function ScheduleButton({ label, overridden, disabled, onClick }: { label: string; overridden: boolean; disabled: boolean; onClick: () => void }) {
  return <button type="button" className="button schedule-button" disabled={disabled} onClick={onClick}>
    {label}{overridden ? ' · Override active' : ''}
  </button>;
}
