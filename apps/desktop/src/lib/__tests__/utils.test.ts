import assert from 'node:assert/strict';

import { describe, it } from 'vitest';

import { cn } from '@/lib/utils';

describe('cn', () => {
  it('merges plain class strings', () => {
    assert.equal(cn('a', 'b'), 'a b');
  });

  it('drops falsy values (undefined, null, empty, false)', () => {
    assert.equal(cn('a', undefined, null, '', false, 'b'), 'a b');
  });

  it('resolves clsx conditional objects and arrays', () => {
    assert.equal(cn('base', { active: true, hidden: false }, ['x', ['y']]), 'base active x y');
  });

  it('dedupes conflicting tailwind classes via twMerge (later wins)', () => {
    // px-2 then px-4 → px-4 wins; p-* is more specific so kept
    assert.equal(cn('px-2', 'px-4'), 'px-4');
    assert.equal(cn('p-1', 'p-2'), 'p-2');
  });

  it('keeps non-conflicting classes untouched', () => {
    assert.equal(cn('text-red-500', 'font-bold'), 'text-red-500 font-bold');
  });

  it('returns empty string for no inputs', () => {
    assert.equal(cn(), '');
  });

  it('returns empty string for only falsy inputs', () => {
    assert.equal(cn(false, undefined, null, ''), '');
  });
});
