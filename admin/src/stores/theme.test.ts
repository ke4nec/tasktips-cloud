import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { beforeEach, describe, expect, it } from 'vitest';

import { useThemeStore } from './theme';

describe('useThemeStore', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
    setActivePinia(createPinia());
  });

  it('defaults to the system mode and applies the resolved theme', () => {
    const store = useThemeStore();
    expect(store.mode).toBe('system');
    expect(['light', 'dark']).toContain(store.resolved);
    expect(document.documentElement.dataset.theme).toBe(store.resolved);
  });

  it('persists an explicit mode and applies it to the document', async () => {
    const store = useThemeStore();
    store.setMode('dark');
    await nextTick();
    expect(store.mode).toBe('dark');
    expect(store.resolved).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(localStorage.getItem('tasktips-admin-theme')).toBe('dark');

    const restored = useThemeStore();
    expect(restored.mode).toBe('dark');
  });

  it('switches back to light from dark', async () => {
    const store = useThemeStore();
    store.setMode('dark');
    await nextTick();
    store.setMode('light');
    await nextTick();
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
