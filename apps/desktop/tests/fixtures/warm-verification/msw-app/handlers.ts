import { http, HttpResponse, type RequestHandler } from 'msw';

import { FixtureStateRegistry, VERIFY_CLIENT_HEADER } from './states';

function clientId(request: Request): string | null {
  return request.headers.get(VERIFY_CLIENT_HEADER);
}

function missingClientResponse() {
  return HttpResponse.json(
    { error: 'Verification client state is not installed' },
    { status: 428 }
  );
}

export function createFixtureHandlers(registry: FixtureStateRegistry): RequestHandler[] {
  return [
    http.get('*/api/portfolio', ({ request }) => {
      const id = clientId(request);
      const state = id ? registry.read(id) : null;
      return state ? HttpResponse.json(state) : missingClientResponse();
    }),
    http.post('*/api/recurring-investments', async ({ request }) => {
      const id = clientId(request);
      if (!id || !registry.read(id)) return missingClientResponse();
      const body = (await request.json().catch(() => null)) as { amountCents?: unknown } | null;
      if (
        typeof body?.amountCents !== 'number' ||
        !Number.isSafeInteger(body.amountCents) ||
        body.amountCents <= 0
      ) {
        return HttpResponse.json(
          { error: 'amountCents must be a positive integer' },
          { status: 400 }
        );
      }
      const state = registry.createInvestment(id, body.amountCents);
      return state ? HttpResponse.json(state, { status: 201 }) : missingClientResponse();
    }),
  ];
}
