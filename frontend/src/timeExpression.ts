// Mirrors the controller's TimeExpr syntax; the controller still validates ranges.
const ATOM = String.raw`(?:\d{2}:\d{2}|(?:sunrise|sunset)(?:[+-]\d{2}:\d{2})?)`;
const EXPRESSION = new RegExp(String.raw`^(?:${ATOM}|(?:min|max)\(${ATOM}, ${ATOM}\))$`);

export function validTimeExpression(value: string): boolean {
  return EXPRESSION.test(value);
}
