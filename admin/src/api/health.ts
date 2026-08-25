import { apiClient } from './client';
import type { components } from './generated/schema';

export const healthPaths = {
  live: '/health/live',
  ready: '/health/ready',
} as const;

export type HealthResponse = components['schemas']['HealthResponse'];

export async function fetchLiveness(): Promise<HealthResponse> {
  return apiClient.getLiveness();
}

export async function fetchReadiness(): Promise<HealthResponse> {
  return apiClient.getReadiness();
}
