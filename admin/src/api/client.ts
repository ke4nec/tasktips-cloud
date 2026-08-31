import type { components, paths } from './generated/schema';

type LivenessResponse =
  paths['/health/live']['get']['responses'][200]['content']['application/json'];
type ReadinessResponse =
  paths['/health/ready']['get']['responses'][200]['content']['application/json'];
type AdminPageQuery = { limit?: number; offset?: number };
type AdminProjectPageQuery = AdminPageQuery & { search?: string };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly requestId?: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export class ApiClient {
  private accessToken: string | undefined;

  constructor(
    private readonly fetcher: typeof fetch = fetch.bind(globalThis),
  ) {}

  setAccessToken(token: string | undefined): void {
    this.accessToken = token;
  }

  get hasAccessToken(): boolean {
    return Boolean(this.accessToken);
  }

  getLiveness(): Promise<LivenessResponse> {
    return this.get<LivenessResponse>('/health/live');
  }

  getReadiness(): Promise<ReadinessResponse> {
    return this.get<ReadinessResponse>('/health/ready');
  }

  login(
    email: string,
    password: string,
  ): Promise<{ accessToken: string; expiresIn: number }> {
    return this.adminPost('/api/v1/admin/auth/login', {
      email,
      password,
      deviceId: adminDeviceId(email),
    });
  }

  refresh(): Promise<{ accessToken: string; expiresIn: number }> {
    return this.adminPost('/api/v1/admin/auth/refresh', undefined);
  }

  logout(): Promise<void> {
    return this.adminPost('/api/v1/admin/auth/logout', undefined).then(
      () => undefined,
    );
  }

  reauthenticate(
    password: string,
  ): Promise<{ nonce: string; expiresIn: number }> {
    return this.adminPost('/api/v1/admin/auth/re-auth', { password });
  }

  getAdminOverview(): Promise<components['schemas']['AdminOverview']> {
    return this.get('/api/v1/admin/overview');
  }

  getAdminUsers(
    query?: AdminPageQuery,
  ): Promise<components['schemas']['AdminUserList']> {
    return this.get(`/api/v1/admin/users${pageQuery(query)}`);
  }

  getAdminProjects(
    query?: AdminProjectPageQuery,
  ): Promise<components['schemas']['ProjectList']> {
    return this.get(`/api/v1/admin/projects${pageQuery(query)}`);
  }

  getAdminUserProjects(
    userId: string,
    query?: AdminPageQuery,
  ): Promise<components['schemas']['ProjectList']> {
    return this.get(
      `/api/v1/admin/users/${userId}/projects${pageQuery(query)}`,
    );
  }

  getAdminDevices(
    query?: AdminPageQuery,
  ): Promise<components['schemas']['DeviceList']> {
    return this.get(`/api/v1/admin/devices${pageQuery(query)}`);
  }

  getAdminUserDevices(
    userId: string,
    query?: AdminPageQuery,
  ): Promise<components['schemas']['DeviceList']> {
    return this.get(`/api/v1/admin/users/${userId}/devices${pageQuery(query)}`);
  }

  getAdminSyncAttempts(
    query?: AdminPageQuery,
  ): Promise<components['schemas']['SyncAttemptList']> {
    return this.get(`/api/v1/admin/sync-attempts${pageQuery(query)}`);
  }

  getAdminAuditEvents(
    query?: AdminPageQuery,
  ): Promise<components['schemas']['AuditEventList']> {
    return this.get(`/api/v1/admin/audit-events${pageQuery(query)}`);
  }

  getAdminRestoreJobs(
    query?: AdminPageQuery,
  ): Promise<components['schemas']['RestoreJobList']> {
    return this.get(`/api/v1/admin/restores${pageQuery(query)}`);
  }

  getAdminJobs(query?: {
    kind?: components['schemas']['JobMetadata']['kind'];
    status?: components['schemas']['JobMetadata']['status'];
    limit?: number;
    offset?: number;
  }): Promise<components['schemas']['AdminJobList']> {
    const params = new URLSearchParams();
    if (query?.kind) params.set('kind', query.kind);
    if (query?.status) params.set('status', query.status);
    if (query?.limit !== undefined) params.set('limit', String(query.limit));
    if (query?.offset !== undefined) params.set('offset', String(query.offset));
    const suffix = params.toString() ? `?${params.toString()}` : '';
    return this.get(`/api/v1/admin/jobs${suffix}`);
  }

  getAdminOperations(
    query?: AdminPageQuery,
  ): Promise<components['schemas']['AdminOperationList']> {
    return this.get(`/api/v1/admin/operations${pageQuery(query)}`);
  }

  getAdminTrends(days = 30): Promise<components['schemas']['AdminTrendList']> {
    return this.get(`/api/v1/admin/metrics/trends?days=${days}`);
  }

  createAdminRestore(
    projectId: string,
    request: components['schemas']['RestoreRequest'],
  ): Promise<components['schemas']['RestoreJob']> {
    return this.adminPost(
      `/api/v1/admin/projects/${projectId}/restores`,
      request,
    );
  }

  cancelRestore(
    projectId: string,
    restoreId: string,
    reason: string,
  ): Promise<components['schemas']['RestoreJob']> {
    return this.post(
      `/api/v1/projects/${projectId}/restores/${restoreId}/cancel`,
      { reason },
    );
  }

  purgeAdminUser(
    userId: string,
    request: components['schemas']['AccountPurgeRequest'],
    reauthNonce: string,
  ): Promise<components['schemas']['AccountPurgeResponse']> {
    return this.adminPost(`/api/v1/admin/users/${userId}/purge`, request, {
      'X-Reauth-Nonce': reauthNonce,
    });
  }

  private async get<T>(path: string): Promise<T> {
    const response = await this.fetcher(path, {
      credentials: 'include',
      headers: this.headers(),
    });

    if (!response.ok) {
      const errorBody = (await response.json().catch(() => undefined)) as
        { code?: string; message?: string; requestId?: string } | undefined;
      throw new ApiError(
        response.status,
        errorBody?.code ?? 'INTERNAL_ERROR',
        errorBody?.message ??
          `API request failed with status ${response.status}`,
        errorBody?.requestId,
      );
    }

    return (await response.json()) as T;
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await this.fetcher(path, {
      method: 'POST',
      credentials: 'include',
      headers: { ...this.headers(), 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      const errorBody = (await response.json().catch(() => undefined)) as
        { code?: string; message?: string; requestId?: string } | undefined;
      throw new ApiError(
        response.status,
        errorBody?.code ?? 'INTERNAL_ERROR',
        errorBody?.message ??
          `API request failed with status ${response.status}`,
        errorBody?.requestId,
      );
    }
    return (await response.json()) as T;
  }

  private async adminPost<T>(
    path: string,
    body: unknown,
    extraHeaders: Record<string, string> = {},
  ): Promise<T> {
    const response = await this.fetcher(path, {
      method: 'POST',
      credentials: 'include',
      headers: {
        ...this.headers(),
        ...this.adminHeaders(),
        ...extraHeaders,
        'Content-Type': 'application/json',
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      const errorBody = (await response.json().catch(() => undefined)) as
        { code?: string; message?: string; requestId?: string } | undefined;
      throw new ApiError(
        response.status,
        errorBody?.code ?? 'INTERNAL_ERROR',
        errorBody?.message ??
          `API request failed with status ${response.status}`,
        errorBody?.requestId,
      );
    }
    return response.status === 204
      ? (undefined as T)
      : ((await response.json()) as T);
  }

  private headers(): Record<string, string> {
    return {
      Accept: 'application/json',
      ...(this.accessToken
        ? { Authorization: `Bearer ${this.accessToken}` }
        : {}),
    };
  }

  private adminHeaders(): Record<string, string> {
    const origin =
      typeof location === 'object' && location !== null
        ? location.origin
        : undefined;
    return origin ? { Origin: origin } : {};
  }
}

function pageQuery(query?: AdminPageQuery | AdminProjectPageQuery): string {
  if (
    !query ||
    (query.limit === undefined &&
      query.offset === undefined &&
      !('search' in query && query.search))
  )
    return '';
  const params = new URLSearchParams();
  if ('search' in query && query.search) params.set('search', query.search);
  if (query.limit !== undefined) params.set('limit', String(query.limit));
  if (query.offset !== undefined) params.set('offset', String(query.offset));
  return `?${params.toString()}`;
}

export function randomUuid(): string {
  const webCrypto = globalThis.crypto;
  if (typeof webCrypto.randomUUID === 'function') return webCrypto.randomUUID();
  // crypto.randomUUID 仅在安全上下文（HTTPS/localhost）可用；明文 HTTP 的
  // 非 localhost origin 下不存在，但 getRandomValues 仍然可用。
  const bytes = webCrypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) =>
    byte.toString(16).padStart(2, '0'),
  ).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function adminDeviceId(email: string): string {
  const normalizedEmail = email.trim().toLowerCase();
  const key = `tasktips_admin_device_id:${normalizedEmail}`;
  const generated = randomUuid();
  try {
    const existing = globalThis.localStorage?.getItem(key);
    if (existing) return existing;
    globalThis.localStorage?.setItem(key, generated);
  } catch {
    // Private browsing may disable storage; keep the login usable for that session.
  }
  return generated;
}

export const apiClient = new ApiClient();
