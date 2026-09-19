<script setup lang="ts">
import { onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { RouterLink } from 'vue-router';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import { apiClient } from '@/api/client';
import packageJson from '../../package.json';
import { useToast } from '@/composables/useToast';
import { useThemeStore } from '@/stores/theme';

const { t } = useI18n();
const theme = useThemeStore();
const toast = useToast();

const loading = ref(true);
const enabled = ref(false);
const error = ref<string>();
const confirmOpen = ref(false);
const pendingEnabled = ref(false);
const saving = ref(false);
const saveError = ref('');
// Release version: identical wiring to App.vue (VITE_APP_VERSION at build
// time, package.json fallback), kept in sync with the OpenAPI info.version.
const appVersion = import.meta.env.VITE_APP_VERSION ?? packageJson.version;

async function load(): Promise<void> {
  loading.value = true;
  error.value = undefined;
  try {
    const settings = await apiClient.getRegistrationSettings();
    enabled.value = settings.enabled;
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
  } finally {
    loading.value = false;
  }
}

function requestToggle(): void {
  if (loading.value || saving.value) return;
  pendingEnabled.value = !enabled.value;
  saveError.value = '';
  confirmOpen.value = true;
}

async function confirmToggle(): Promise<void> {
  if (saving.value) return;
  saving.value = true;
  saveError.value = '';
  try {
    const settings = await apiClient.updateRegistrationSettings(
      pendingEnabled.value,
    );
    enabled.value = settings.enabled;
    confirmOpen.value = false;
    toast.show(t('common.saved'));
  } catch (reason) {
    // Keep the switch on the server value; surface the error in the dialog.
    saveError.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section>
    <header class="page-heading">
      <div>
        <div class="eyebrow">{{ t('eyebrow.settings') }}</div>
        <h1>{{ t('title.settings') }}</h1>
        <p>{{ t('description.settings') }}</p>
      </div>
    </header>

    <div v-if="error" class="notice danger" style="margin-bottom: 16px">
      <AppIcon name="alert" />{{ error }}
    </div>

    <div v-if="loading" class="panel state-panel state-compact">
      <div class="skeleton skeleton-title"></div>
      <div class="skeleton"></div>
      <div class="skeleton"></div>
    </div>
    <div v-else class="settings-layout">
      <div class="settings-main">
        <section class="panel settings-panel">
          <h2><AppIcon name="users" />{{ t('settings.access') }}</h2>
          <div class="setting-row">
            <div>
              <strong>{{ t('settings.registration') }}</strong>
              <p>{{ t('settings.registrationHint') }}</p>
            </div>
            <button
              class="switch"
              :class="{ on: enabled }"
              role="switch"
              :aria-checked="enabled"
              :aria-label="t('settings.registration')"
              @click="requestToggle"
            >
              <span></span>
            </button>
          </div>
          <div class="panel-footnote">
            <AppIcon name="info" />{{ t('settings.approval') }}
          </div>
        </section>

        <section class="panel settings-panel">
          <h2><AppIcon name="sun" />{{ t('settings.appearance') }}</h2>
          <div class="setting-row">
            <div>
              <strong>{{ t('settings.theme') }}</strong>
              <p>{{ t('settings.themeHint') }}</p>
            </div>
            <div class="segmented">
              <button
                v-for="mode in ['light', 'dark', 'system'] as const"
                :key="mode"
                :class="{ selected: theme.mode === mode }"
                :aria-pressed="theme.mode === mode"
                @click="theme.setMode(mode)"
              >
                <AppIcon
                  :name="
                    mode === 'light'
                      ? 'sun'
                      : mode === 'dark'
                        ? 'moon'
                        : 'monitor'
                  "
                />{{ t(`settings.${mode}`) }}
              </button>
            </div>
          </div>
          <div class="setting-row">
            <div>
              <strong>{{ t('settings.language') }}</strong>
              <p>{{ t('settings.languageHint') }}</p>
            </div>
            <span>{{ t('settings.zh') }}</span>
          </div>
        </section>

        <section class="panel settings-panel">
          <h2><AppIcon name="cloud" />{{ t('settings.about') }}</h2>
          <div class="setting-row">
            <span>{{ t('settings.version') }}</span>
            <code>v{{ appVersion }}</code>
          </div>
          <div class="setting-row">
            <span>{{ t('settings.mode') }}</span>
            <span>{{ t('brand.selfHosted') }}</span>
          </div>
        </section>
      </div>
      <aside class="settings-aside">
        <span class="aside-icon"><AppIcon name="shield" /></span>
        <h3>{{ t('privacy.title') }}</h3>
        <p>{{ t('privacy.settings') }}</p>
        <RouterLink to="/audit"
          >{{ t('settings.audit') }}<AppIcon name="arrow-right"
        /></RouterLink>
      </aside>
    </div>

    <AppDialog :open="confirmOpen" @close="confirmOpen = false">
      <template v-if="confirmOpen">
        <header class="modal-header">
          <div class="modal-heading-icon"><AppIcon name="shield" /></div>
          <button
            class="icon-button"
            :aria-label="t('common.close')"
            @click="confirmOpen = false"
          >
            <AppIcon name="close" />
          </button>
        </header>
        <h2>{{ t('modal.registration') }}</h2>
        <p class="muted modal-description">{{ t('modalHint.registration') }}</p>
        <div class="notice info">
          <AppIcon name="info" />{{
            pendingEnabled
              ? t('settings.openImpact')
              : t('settings.closeImpact')
          }}
        </div>
        <p v-if="saveError" class="form-error" role="alert">{{ saveError }}</p>
        <footer class="modal-footer">
          <button
            type="button"
            class="button"
            :disabled="saving"
            @click="confirmOpen = false"
          >
            {{ t('common.cancel') }}
          </button>
          <button
            class="button primary"
            :disabled="saving"
            @click="confirmToggle"
          >
            {{ t('common.confirm') }}
          </button>
        </footer>
      </template>
    </AppDialog>
  </section>
</template>
