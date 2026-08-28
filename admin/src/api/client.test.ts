import { describe, expect, it, vi } from 'vitest';

import { ApiClient, randomUuid } from './client';

describe('randomUuid', () => {
  it('uses crypto.randomUUID in secure contexts', () => {
    const spy = vi
      .spyOn(crypto, 'randomUUID')
      .mockReturnValue('cf65cb63-93f2-4dc6-9d77-91ee45d16a09');
    expect(randomUuid()).toBe('cf65cb63-93f2-4dc6-9d77-91ee45d16a09');
    spy.mockRestore();
  });

  it('falls back to getRandomValues outside secure contexts', () => {
    const secureContextCrypto = crypto;
    vi.stubGlobal('crypto', {
      getRandomValues:
        secureContextCrypto.getRandomValues.bind(secureContextCrypto),
    });
    try {
      const uuid = randomUuid();
      expect(uuid).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
      );
      expect(randomUuid()).not.toBe(uuid);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe('ApiClient', () => {
  it('binds the default fetcher to the global context', async () => {
    const strictThisStub = async function (this: unknown): Promise<Response> {
      // 未绑定时 this 是 ApiClient 实例或 undefined；真实浏览器会抛
      // "'fetch' called on an object that does not implement interface Window"。
      if (this !== globalThis) {
        throw new Error('fetch invoked without the global this');
      }
      return new Response(
        JSON.stringify({ status: 'live', database: null, objectStore: null }),
        { headers: { 'content-type': 'application/json' } },
      );
    };
    vi.stubGlobal('fetch', strictThisStub);
    try {
      await expect(new ApiClient().getLiveness()).resolves.toEqual({
        status: 'live',
        database: null,
        objectStore: null,
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });
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
      credentials: 'include',
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

  it('uses the isolated administrator login path and reuses its device id', async () => {
    localStorage.clear();
    const fetcher = vi.fn<typeof fetch>().mockImplementation(
      async () =>
        new Response(
          JSON.stringify({ accessToken: 'access', expiresIn: 900 }),
          {
            headers: { 'content-type': 'application/json' },
          },
        ),
    );
    const client = new ApiClient(fetcher);

    await client.login('admin@example.test', 'password-one');
    await client.login('admin@example.test', 'password-two');

    const firstRequest = fetcher.mock.calls[0]?.[1];
    const secondRequest = fetcher.mock.calls[1]?.[1];
    const firstBody = JSON.parse(String(firstRequest?.body)) as {
      deviceId: string;
    };
    const secondBody = JSON.parse(String(secondRequest?.body)) as {
      deviceId: string;
    };
    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      '/api/v1/admin/auth/login',
      expect.objectContaining({ method: 'POST', credentials: 'include' }),
    );
    expect(firstBody.deviceId).toBe(secondBody.deviceId);
    expect(
      localStorage.getItem('tasktips_admin_device_id:admin@example.test'),
    ).toBe(firstBody.deviceId);
  });

  it('uses the administrator cookie endpoints for refresh and logout', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ accessToken: 'access', expiresIn: 900 }),
          {
            headers: { 'content-type': 'application/json' },
          },
        ),
      )
      .mockResolvedValueOnce(new Response(undefined, { status: 204 }));
    const client = new ApiClient(fetcher);

    await client.refresh();
    client.setAccessToken('access');
    await client.logout();

    expect(fetcher.mock.calls[0]?.[0]).toBe('/api/v1/admin/auth/refresh');
    expect(fetcher.mock.calls[1]?.[0]).toBe('/api/v1/admin/auth/logout');
  });

  it('uses globally paginated administrator project and device paths', async () => {
    const fetcher = vi.fn<typeof fetch>().mockImplementation(
      async () =>
        new Response(JSON.stringify({ items: [], hasMore: false }), {
          headers: { 'content-type': 'application/json' },
        }),
    );
    const client = new ApiClient(fetcher);

    await client.getAdminProjects({ limit: 25, offset: 50 });
    await client.getAdminDevices({ limit: 25, offset: 50 });

    expect(fetcher.mock.calls[0]?.[0]).toBe(
      '/api/v1/admin/projects?limit=25&offset=50',
    );
    expect(fetcher.mock.calls[1]?.[0]).toBe(
      '/api/v1/admin/devices?limit=25&offset=50',
    );
  });

  it('sends the administrator origin for restore requests', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ status: 'queued' }), {
        headers: { 'content-type': 'application/json' },
      }),
    );
    const client = new ApiClient(fetcher);
    client.setAccessToken('admin-access');

    await client.createAdminRestore('project-1', {
      targetChangeSequence: 12,
      reason: 'verified restore target',
    });

    expect(fetcher).toHaveBeenCalledWith(
      '/api/v1/admin/projects/project-1/restores',
      expect.objectContaining({
        method: 'POST',
        credentials: 'include',
        headers: expect.objectContaining({
          Authorization: 'Bearer admin-access',
          Origin: location.origin,
        }),
      }),
    );
  });

  it('filters safe administrator jobs metadata through the generated contract', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ items: [] }), {
        headers: { 'content-type': 'application/json' },
      }),
    );
    const client = new ApiClient(fetcher);
    await client.getAdminJobs({ kind: 'project_purge', status: 'queued' });

    expect(fetcher).toHaveBeenCalledWith(
      '/api/v1/admin/jobs?kind=project_purge&status=queued',
      expect.objectContaining({ credentials: 'include' }),
    );
  });

  it('sends the one-use re-auth nonce for account purge confirmation', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          jobId: 'job-1',
          exportId: 'export-1',
          status: 'queued',
          confirmationRequired: false,
        }),
        { headers: { 'content-type': 'application/json' } },
      ),
    );
    const client = new ApiClient(fetcher);
    client.setAccessToken('admin-access');

    await client.purgeAdminUser(
      'user-1',
      { confirmed: true, exportId: 'export-1', reason: 'verified request' },
      'reauth-nonce',
    );

    expect(fetcher).toHaveBeenCalledWith(
      '/api/v1/admin/users/user-1/purge',
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: 'Bearer admin-access',
          Origin: location.origin,
          'X-Reauth-Nonce': 'reauth-nonce',
        }),
      }),
    );
  });
});
