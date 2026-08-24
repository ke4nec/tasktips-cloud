import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

import { fetchLiveness } from '@/api/health';

export type HealthState = 'checking' | 'live' | 'unavailable';

export const useHealthStore = defineStore('health', () => {
  const state = ref<HealthState>('checking');
  const isLive = computed(() => state.value === 'live');

  async function check(): Promise<void> {
    state.value = 'checking';
    try {
      const response = await fetchLiveness();
      state.value = response.status === 'live' ? 'live' : 'unavailable';
    } catch {
      state.value = 'unavailable';
    }
  }

  return { state, isLive, check };
});

export function healthLabelKey(state: HealthState): string {
  if (state === 'live') return 'app.apiLive';
  if (state === 'unavailable') return 'app.apiUnavailable';
  return 'app.checking';
}
