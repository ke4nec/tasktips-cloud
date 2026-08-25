import { describe, expect, it, vi } from 'vitest';

import { ApiClient } from './client';

describe('ApiClient', () => {
  it('uses the generated liveness path and response type', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          status: 'live',
          database: null,
          objectStore: null,
        }),
        { headers: { 'content-type': 'application/json' } },
      ),
    );

    const client = new ApiClient(fetcher);
    await expect(client.getLiveness()).resolves.toEqual({
      status: 'live',
      database: null,
      objectStore: null,
    });
    expect(fetcher).toHaveBeenCalledWith('/health/live', {
      headers: { Accept: 'application/json' },
    });
  });

  it('maps a contract error response to ApiError', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          code: 'NOT_FOUND',
          message: '资源不存在',
          retryable: false,
          requestId: 'request-1',
        }),
        {
          status: 404,
          headers: { 'content-type': 'application/json' },
        },
      ),
    );

    await expect(new ApiClient(fetcher).getLiveness()).rejects.toMatchObject({
      status: 404,
      code: 'NOT_FOUND',
      message: '资源不存在',
      requestId: 'request-1',
    });
  });
});
