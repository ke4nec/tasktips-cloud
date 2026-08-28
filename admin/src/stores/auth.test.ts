import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { apiClient } from '@/api/client';

import { useAuthStore } from './auth';

describe('useAuthStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    apiClient.setAccessToken(undefined);
    vi.restoreAllMocks();
  });

  it('updates the reactive authentication state after login', async () => {
    vi.spyOn(apiClient, 'login').mockResolvedValue({
      accessToken: 'access-token',
      expiresIn: 900,
    });
    const store = useAuthStore();

    expect(store.authenticated).toBe(false);
    await expect(store.login('admin@example.com', 'password')).resolves.toBe(
      true,
    );

    expect(store.authenticated).toBe(true);
    expect(apiClient.hasAccessToken).toBe(true);
  });

  it('clears a stale token when refresh fails', async () => {
    apiClient.setAccessToken('stale-token');
    vi.spyOn(apiClient, 'refresh').mockRejectedValue(new Error('expired'));
    const store = useAuthStore();

    expect(store.authenticated).toBe(true);
    await store.restore();

    expect(store.authenticated).toBe(false);
    expect(apiClient.hasAccessToken).toBe(false);
  });

  it('clears the reactive authentication state after logout', async () => {
    apiClient.setAccessToken('access-token');
    vi.spyOn(apiClient, 'logout').mockResolvedValue();
    const store = useAuthStore();

    await store.logout();

    expect(store.authenticated).toBe(false);
    expect(apiClient.hasAccessToken).toBe(false);
  });
});
