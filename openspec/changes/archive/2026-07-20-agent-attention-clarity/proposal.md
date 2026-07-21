# Agent attention clarity

## Why

The Work surface can already detect some yellow states, but it presents them as a generic status and small inline controls. When Codex or Claude is waiting for approval, confirmation, an answer, or setup, the user can miss the action and mistake a blocked run for a healthy one.

## What Changes

Add a provider-aware attention presentation to Work. Confirmed structured events must show a prominent reason and one clear next action. Direct terminal heuristics may raise only a clearly labelled possible prompt. The surface must also make attention visible when the run is not currently selected, without bypassing provider permissions.

Out of scope: autonomous approval, parsing raw terminal output into fake chat messages, or changing provider launch policies.

## Success

Within one glance, a user can answer: is the agent working, what does it need, how certain is that diagnosis, and what can I do next?
