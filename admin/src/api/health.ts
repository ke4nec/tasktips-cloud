import type { components } from './generated/schema';

export const healthPaths = {
  live: '/health/live',
  ready: '/health/ready',
} as const;

export type HealthResponse = components['schemas']['HealthResponse'];

export async function fetchLiveness(): Promise<HealthResponse> {
  const response = await fetch(healthPaths.live, {
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) {
    throw new Error(`health check failed with status ${response.status}`);
  }
  return (await response.json()) as HealthResponse;
}
