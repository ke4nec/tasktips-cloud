import { defineStore } from 'pinia';
import { computed, ref, watchEffect } from 'vue';

export type ThemeMode = 'light' | 'dark' | 'system';

const STORAGE_KEY = 'tasktips-admin-theme';

function readStoredMode(): ThemeMode {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'light' || stored === 'dark' || stored === 'system')
      return stored;
  } catch {
    // Storage may be unavailable (private browsing); fall back to system.
  }
  return 'system';
}

export const useThemeStore = defineStore('theme', () => {
  const mode = ref<ThemeMode>(readStoredMode());
  const systemPrefersDark = ref(
    typeof matchMedia === 'function'
      ? matchMedia('(prefers-color-scheme: dark)').matches
      : false,
  );
  const resolved = computed(() =>
    mode.value === 'system'
      ? systemPrefersDark.value
        ? 'dark'
        : 'light'
      : mode.value,
  );

  if (typeof matchMedia === 'function') {
    matchMedia('(prefers-color-scheme: dark)').addEventListener(
      'change',
      (event) => {
        systemPrefersDark.value = event.matches;
      },
    );
  }

  watchEffect(() => {
    document.documentElement.dataset.theme = resolved.value;
  });

  function setMode(next: ThemeMode): void {
    mode.value = next;
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Keep the in-session theme even when persistence is unavailable.
    }
  }

  return { mode, resolved, setMode };
});
