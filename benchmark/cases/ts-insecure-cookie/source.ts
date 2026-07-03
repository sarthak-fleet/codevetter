// Case: Session cookie set without Secure, HttpOnly, or SameSite attributes.
import type { Response } from 'express';

export function setSessionCookie(res: Response, token: string): void {
  // BUG: the cookie is set without Secure (sent over HTTP), HttpOnly (readable
  // by JS/XSS), and SameSite (vulnerable to CSRF). A stolen cookie value is a
  // stolen session.
  res.cookie('session', token, { maxAge: 86400000 });
}
