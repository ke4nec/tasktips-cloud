import type { components, paths } from './generated/schema';

type LivenessResponse =
  paths['/health/live']['get']['responses'][200]['content']['application/json'];
type ReadinessResponse =
  paths['/health/ready']['get']['responses'][200]['content']['application/json'];

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

  constructor(private readonly fetcher: typeof fetch = fetch) {}

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

  login(email: string, password: string): Promise<{ accessToken: string; expiresIn: number }> {
    return this.post('/api/v1/auth/login', { email, password, deviceId: crypto.randomUUID() });
  }

  refresh(): Promise<{ accessToken: string; expiresIn: number }> {
    return this.post('/api/v1/auth/refresh', undefined);
  }

  logout(): Promise<void> {
    return this.post('/api/v1/auth/logout', undefined).then(() => undefined);
  }

  getAdminOverview(): Promise<components['schemas']['AdminOverview']> {
    return this.get('/api/v1/admin/overview');
  }

  getAdminUsers(): Promise<components['schemas']['AdminUserList']> {
    return this.get('/api/v1/admin/users');
  }

  getAdminProjects(userId?: string): Promise<components['schemas']['ProjectList']> {
    return this.get(userId ? `/api/v1/admin/users/${userId}/projects` : '/api/v1/projects');
  }

  getAdminDevices(userId?: string): Promise<components['schemas']['DeviceList']> {
    return this.get(userId ? `/api/v1/admin/users/${userId}/devices` : '/api/v1/devices');
  }

  getAdminSyncAttempts(): Promise<components['schemas']['SyncAttemptList']> {
    return this.get('/api/v1/admin/sync-attempts');
  }

  getAdminAuditEvents(): Promise<components['schemas']['AuditEventList']> {
    return this.get('/api/v1/admin/audit-events');
  }

  getAdminRestoreJobs(): Promise<components['schemas']['RestoreJobList']> {
    return this.get('/api/v1/admin/restores');
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
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      const errorBody = (await response.json().catch(() => undefined)) as
        { code?: string; message?: string; requestId?: string } | undefined;
      throw new ApiError(
        response.status,
        errorBody?.code ?? 'INTERNAL_ERROR',
        errorBody?.message ?? `API request failed with status ${response.status}`,
        errorBody?.requestId,
      );
    }
    return response.status === 204 ? (undefined as T) : ((await response.json()) as T);
  }

  private headers(): Record<string, string> {
    return {
      Accept: 'application/json',
      ...(this.accessToken ? { Authorization: `Bearer ${this.accessToken}` } : {}),
    };
  }
}

export const apiClient = new ApiClient();
