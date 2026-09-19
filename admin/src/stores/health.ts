import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

import { fetchLiveness, fetchReadiness } from '@/api/health';

export type HealthState = 'checking' | 'live' | 'unavailable';
export type ReadinessState = 'checking' | 'ready' | 'notReady';

export const useHealthStore = defineStore('health', () => {
  const state = ref<HealthState>('checking');
  const readiness = ref<ReadinessState>('checking');
  const database = ref<boolean | null>(null);
  const objectStore = ref<boolean | null>(null);
  const checkedAt = ref<string | null>(null);
  const isLive = computed(() => state.value === 'live');

  async function check(): Promise<void> {
    state.value = 'checking';
    readiness.value = 'checking';
    const [liveness, ready] = await Promise.allSettled([
      fetchLiveness(),
      fetchReadiness(),
    ]);
    if (liveness.status === 'fulfilled') {
      state.value = liveness.value.status === 'live' ? 'live' : 'unavailable';
    } else {
      state.value = 'unavailable';
    }
    if (ready.status === 'fulfilled') {
      readiness.value = ready.value.status === 'ready' ? 'ready' : 'notReady';
      database.value = ready.value.database;
      objectStore.value = ready.value.objectStore;
    } else {
      readiness.value = 'notReady';
      database.value = null;
      objectStore.value = null;
    }
    checkedAt.value = new Date().toLocaleTimeString('zh-CN', { hour12: false });
  }
  return { state, readiness, database, objectStore, checkedAt, isLive, check };
});

export function healthLabelKey(state: HealthState): string {
  if (state === 'live') return 'app.apiLive';
  if (state === 'unavailable') return 'app.apiUnavailable';
  return 'app.checking';
}
