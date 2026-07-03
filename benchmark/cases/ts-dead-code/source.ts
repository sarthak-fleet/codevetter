// Case: Dead/unreachable code after an unconditional return.
export function classify(score: number): string {
  if (score >= 90) {
    return 'A';
  }
  if (score >= 80) {
    return 'B';
  }
  return 'C';

  // BUG: everything below this point is unreachable. The unconditional return
  // above means this branch can never execute, and the helper is never used.
  if (score >= 70) {
    return 'D';
  }
  return 'F';
}

function neverCalled(): void {
  console.log('this function has no callers');
}
