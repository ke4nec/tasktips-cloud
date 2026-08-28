import { defineStore } from 'pinia';
import { ref } from 'vue';

import { apiClient } from '@/api/client';

export const useAuthStore = defineStore('auth', () => {
  const ready = ref(false);
  const loading = ref(false);
  const error = ref<string | undefined>();
  // Keep authentication reactive; ApiClient's token is intentionally private
  // mutable state and cannot invalidate a Vue computed value by itself.
  const authenticated = ref(apiClient.hasAccessToken);

  function setAccessToken(token: string | undefined): void {
    apiClient.setAccessToken(token);
    authenticated.value = Boolean(token);
  }

  async function restore(): Promise<void> {
    try {
      const tokens = await apiClient.refresh();
      setAccessToken(tokens.accessToken);
    } catch {
      setAccessToken(undefined);
    } finally {
      ready.value = true;
    }
  }

  async function login(email: string, password: string): Promise<boolean> {
    loading.value = true;
    error.value = undefined;
    try {
      const tokens = await apiClient.login(email, password);
      setAccessToken(tokens.accessToken);
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
      setAccessToken(undefined);
    }
  }

  return { ready, loading, error, authenticated, restore, login, logout };
});
