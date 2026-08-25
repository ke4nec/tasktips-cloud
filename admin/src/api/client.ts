import type { paths } from './generated/schema';

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
  constructor(private readonly fetcher: typeof fetch = fetch) {}

  getLiveness(): Promise<LivenessResponse> {
    return this.get<LivenessResponse>('/health/live');
  }

  getReadiness(): Promise<ReadinessResponse> {
    return this.get<ReadinessResponse>('/health/ready');
  }

  private async get<T>(path: string): Promise<T> {
    const response = await this.fetcher(path, {
      headers: { Accept: 'application/json' },
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
}

export const apiClient = new ApiClient();
