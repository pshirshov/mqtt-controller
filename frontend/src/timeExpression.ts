// Mirrors the controller's TimeExpr syntax; the controller still validates ranges.
const ATOM = String.raw`(?:\d{2}:\d{2}|(?:sunrise|sunset)(?:[+-]\d{2}:\d{2})?)`;
const EXPRESSION = new RegExp(String.raw`^(?:${ATOM}|(?:min|max)\(${ATOM}, ${ATOM}\))$`);
const FIXED = /^(\d{2}):(\d{2})$/;
const SUN = /^(sunrise|sunset)(?:([+-])(\d{2}):(\d{2}))?$/;
const MIN_MAX = /^(min|max)\((.+), (.+)\)$/;
export const MINUTES_PER_DAY = 1440;
// Nominal sun times used only to order slots for display.
const NOMINAL_SUNRISE = 6 * 60;
const NOMINAL_SUNSET = 18 * 60;

export function validTimeExpression(value: string): boolean {
  return EXPRESSION.test(value);
}

/** Minutes since midnight of a fixed `HH:MM` expression, `null` for anything else. */
export function fixedMinutes(value: string): number | null {
  const match = FIXED.exec(value);
  if (match === null) return null;
  const minutes = Number(match[1]) * 60 + Number(match[2]);
  return Number(match[2]) < 60 && minutes <= MINUTES_PER_DAY ? minutes : null;
}

/** Minutes since midnight assuming nominal 06:00 sunrise and 18:00 sunset; for ordering only. */
export function nominalMinutes(value: string): number {
  const fixed = fixedMinutes(value);
  if (fixed !== null) return fixed;
  const sun = SUN.exec(value);
  if (sun !== null) {
    const base = sun[1] === 'sunrise' ? NOMINAL_SUNRISE : NOMINAL_SUNSET;
    const offset = sun[2] === undefined ? 0 : (Number(sun[3]) * 60 + Number(sun[4])) * (sun[2] === '-' ? -1 : 1);
    return Math.min(MINUTES_PER_DAY, Math.max(0, base + offset));
  }
  const combined = MIN_MAX.exec(value);
  if (combined !== null) {
    const [a, b] = [nominalMinutes(combined[2]!), nominalMinutes(combined[3]!)];
    return combined[1] === 'min' ? Math.min(a, b) : Math.max(a, b);
  }
  return MINUTES_PER_DAY;
}

export function formatMinutes(minutes: number): string {
  return `${String(Math.floor(minutes / 60)).padStart(2, '0')}:${String(minutes % 60).padStart(2, '0')}`;
}
