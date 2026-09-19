import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { healthLabelKey, useHealthStore } from './health';

const { fetchLiveness, fetchReadiness } = vi.hoisted(() => ({
  fetchLiveness: vi.fn(),
  fetchReadiness: vi.fn(),
}));

vi.mock('@/api/health', () => ({
  fetchLiveness: (...args: unknown[]) => fetchLiveness(...args),
  fetchReadiness: (...args: unknown[]) => fetchReadiness(...args),
}));

describe('useHealthStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    fetchLiveness.mockReset();
    fetchReadiness.mockReset();
  });

  it('reflects readiness and connection details from /health/ready', async () => {
    fetchLiveness.mockResolvedValue({ status: 'live' });
    fetchReadiness.mockResolvedValue({
      status: 'ready',
      database: true,
      objectStore: false,
    });

    const store = useHealthStore();
    await store.check();

    expect(store.isLive).toBe(true);
    expect(store.readiness).toBe('ready');
    expect(store.database).toBe(true);
    expect(store.objectStore).toBe(false);
    expect(store.checkedAt).toBeTruthy();
  });

  it('falls back to notReady with unknown connections when readiness fails', async () => {
    fetchLiveness.mockResolvedValue({ status: 'live' });
    fetchReadiness.mockRejectedValue(new Error('down'));

    const store = useHealthStore();
    await store.check();

    expect(store.isLive).toBe(true);
    expect(store.readiness).toBe('notReady');
    expect(store.database).toBeNull();
    expect(store.objectStore).toBeNull();
  });

  it('still reports readiness details when liveness fails', async () => {
    fetchLiveness.mockRejectedValue(new Error('down'));
    fetchReadiness.mockResolvedValue({
      status: 'notReady',
      database: false,
      objectStore: null,
    });

    const store = useHealthStore();
    await store.check();

    expect(store.isLive).toBe(false);
    expect(store.readiness).toBe('notReady');
    expect(store.database).toBe(false);
    expect(store.objectStore).toBeNull();
  });
});

describe('healthLabelKey', () => {
  it('maps every health state to an i18n key', () => {
    expect(healthLabelKey('checking')).toBe('app.checking');
    expect(healthLabelKey('live')).toBe('app.apiLive');
    expect(healthLabelKey('unavailable')).toBe('app.apiUnavailable');
  });
});
