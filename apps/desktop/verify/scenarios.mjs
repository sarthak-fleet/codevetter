export const scenarioModule = {
  id: 'codevetter-shell',
  scenarios: [
    {
      schemaVersion: 1,
      id: 'shell-navigation',
      capabilityIds: ['app-shell'],
      route: '/',
      authProfileId: 'local-developer',
      stateName: 'shell-navigation-ready',
      frozenTime: '2026-07-15T10:00:00.000Z',
      flags: {},
      timeouts: { actionMs: 3000, scenarioMs: 10000 },
      actions: [
        { id: 'open-trex', kind: 'click', description: 'Open T-Rex from the shell' },
        { id: 'verify-trex', kind: 'wait', description: 'Wait for the T-Rex route' },
        { id: 'return-home', kind: 'click', description: 'Return to Home' },
      ],
      assertions: [
        { id: 'trex-route', kind: 'route', description: 'T-Rex opens directly' },
        {
          id: 'shell-visible',
          kind: 'visible',
          description: 'The CodeVetter shell remains visible',
        },
        { id: 'runtime-clean', kind: 'runtime_errors', description: 'No runtime error occurs' },
      ],
      async run({ page, observe, step }) {
        await step('open-trex', () => page.getByRole('link', { name: /T-Rex/i }).click());
        await step('verify-trex', () => observe.expectRoute('/trex'));
        await observe.expectVisible('T-Rex watcher');
        await step('return-home', () => page.getByRole('link', { name: /Home/i }).click());
        await observe.expectRoute('/');
        await observe.expectVisible('CodeVetter');
        await observe.expectNoRuntimeErrors();
      },
    },
  ],
};
