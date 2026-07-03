// Case: Prototype pollution via recursive object merge.
function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null;
}

// BUG: the merge walks user-supplied keys without blocking __proto__,
// constructor, or prototype. A payload like {"__proto__": {"admin": true}}
// pollutes Object.prototype and escalates privileges app-wide.
export function merge(target: Record<string, unknown>, source: unknown): Record<string, unknown> {
  if (!isObject(source)) return target;
  for (const key of Object.keys(source)) {
    const tv = target[key];
    const sv = source[key];
    if (isObject(tv) && isObject(sv)) {
      merge(tv, sv);
    } else {
      target[key] = sv;
    }
  }
  return target;
}
