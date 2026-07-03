// Case: Catastrophic backtracking regex (ReDoS).
// This regex is used to validate user-supplied email-like strings.
export const emailLikePattern = /^([a-zA-Z0-9._%+-]+)+$/;

// BUG: the nested + quantifier ((...+)+) creates exponential backtracking on
// non-matching inputs. A long string like "a".repeat(30) + "!" hangs the event
// loop and denies service to all other requests.
export function isEmailLike(input: string): boolean {
  return emailLikePattern.test(input);
}
