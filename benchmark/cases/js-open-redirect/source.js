// Case: Open redirect via unvalidated user-controlled URL.
const express = require('express');
const app = express();

app.get('/login', (req, res) => {
  // BUG: the `next` query param is used directly in res.redirect without any
  // allowlist or same-origin check, so an attacker can craft
  // /login?next=https://evil.example to phish users off the trusted domain.
  const next = req.query.next;
  if (next) {
    return res.redirect(next);
  }
  res.redirect('/dashboard');
});
