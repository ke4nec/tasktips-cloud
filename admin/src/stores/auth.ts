import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

import { apiClient } from '@/api/client';

export const useAuthStore = defineStore('auth', () => {
  const ready = ref(false);
  const loading = ref(false);
  const error = ref<string | undefined>();
  const authenticated = computed(() => apiClient.hasAccessToken);

  async function restore(): Promise<void> {
    try {
      const tokens = await apiClient.refresh();
      apiClient.setAccessToken(tokens.accessToken);
    } catch {
      apiClient.setAccessToken(undefined);
    } finally {
      ready.value = true;
    }
  }

  async function login(email: string, password: string): Promise<boolean> {
    loading.value = true;
    error.value = undefined;
    try {
      const tokens = await apiClient.login(email, password);
      apiClient.setAccessToken(tokens.accessToken);
      return true;
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : '登录失败';
      return false;
    } finally {
      loading.value = false;
      ready.value = true;
    }
  }

  async function logout(): Promise<void> {
    try {
      await apiClient.logout();
    } finally {
      apiClient.setAccessToken(undefined);
    }
  }

  return { ready, loading, error, authenticated, restore, login, logout };
});
