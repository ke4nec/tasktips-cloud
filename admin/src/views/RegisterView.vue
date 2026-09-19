<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { RouterLink } from 'vue-router';

import AppIcon from '@/components/AppIcon.vue';
import brandIcon from '@/assets/tasktips.png';
import { ApiError, apiClient } from '@/api/client';
import { isValidPassword } from '@/lib/format';

const { t } = useI18n();
const email = ref('');
const password = ref('');
const confirm = ref('');
const loading = ref(false);
const checking = ref(true);
const registrationOpen = ref(false);
const error = ref<string>();
const succeeded = ref(false);
const showPassword = ref(false);
const canSubmit = computed(
  () =>
    email.value.trim().length > 0 &&
    isValidPassword(password.value) &&
    confirm.value === password.value,
);

onMounted(async () => {
  try {
    const status = await apiClient.registrationStatus();
    registrationOpen.value = status.enabled;
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
  } finally {
    checking.value = false;
  }
});

async function submit(): Promise<void> {
  if (!canSubmit.value || loading.value) return;
  loading.value = true;
  error.value = undefined;
  try {
    await apiClient.register(email.value.trim(), password.value);
    succeeded.value = true;
  } catch (reason) {
    error.value =
      reason instanceof ApiError ? reason.message : t('common.loadFailed');
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <main class="auth-page">
    <span class="brand auth-brand">
      <img :src="brandIcon" alt="" />
      <strong>{{ t('brand.name') }}</strong>
    </span>
    <div class="auth-layout">
      <section class="auth-story">
        <span class="eyebrow">{{ t('auth.eyebrow') }}</span>
        <h1>
          {{ t('auth.hero1') }}<br /><span>{{ t('auth.hero2') }}</span>
        </h1>
        <p>{{ t('auth.heroDescription') }}</p>
        <div class="auth-trust">
          <AppIcon name="shield" /><span>{{ t('auth.trust') }}</span>
        </div>
      </section>
      <section class="auth-card panel">
        <template v-if="succeeded">
          <div class="success-mark"><AppIcon name="check" /></div>
          <h2>{{ t('auth.registered') }}</h2>
          <p>{{ t('auth.registeredHint') }}</p>
          <RouterLink class="button primary full-width" to="/login">
            {{ t('auth.backToLogin') }}
          </RouterLink>
        </template>
        <template v-else-if="checking">
          <div class="skeleton skeleton-title"></div>
          <div class="skeleton"></div>
          <div class="skeleton"></div>
        </template>
        <template v-else-if="!registrationOpen">
          <div class="empty-icon"><AppIcon name="lock" /></div>
          <h2>{{ t('auth.closed') }}</h2>
          <p>{{ t('auth.closedHint') }}</p>
          <RouterLink class="button full-width" to="/login">
            {{ t('auth.backToLogin') }}
          </RouterLink>
        </template>
        <template v-else>
          <div class="auth-card-top">
            <span class="auth-kicker">{{ t('auth.join') }}</span>
            <AppIcon name="lock" />
          </div>
          <h2>{{ t('auth.createAccount') }}</h2>
          <p>{{ t('auth.registerHint') }}</p>
          <form @submit.prevent="submit">
            <label
              >{{ t('auth.email') }}
              <input
                v-model="email"
                type="email"
                :placeholder="t('auth.emailPlaceholder')"
                autocomplete="username"
                required
            /></label>
            <label
              >{{ t('auth.password') }}
              <div class="password-field">
                <input
                  v-model="password"
                  :type="showPassword ? 'text' : 'password'"
                  :placeholder="t('auth.passwordPlaceholder')"
                  :minlength="12"
                  autocomplete="new-password"
                  required
                /><button
                  type="button"
                  class="icon-button"
                  :aria-label="t('auth.showPassword')"
                  @click="showPassword = !showPassword"
                >
                  <AppIcon name="eye" />
                </button></div
            ></label>
            <small class="field-hint">{{ t('auth.passwordHint') }}</small>
            <label
              >{{ t('auth.confirmPassword') }}
              <input
                v-model="confirm"
                type="password"
                autocomplete="new-password"
                required
            /></label>
            <div v-if="error" class="notice danger" role="alert">
              <AppIcon name="alert" />{{ error }}
            </div>
            <button
              class="button primary full-width auth-submit"
              :disabled="loading"
            >
              {{ t('auth.submitRegister') }}<AppIcon name="arrow-right" />
            </button>
          </form>
          <div class="auth-switch">
            <span>{{ t('auth.hasAccount') }}</span>
            <RouterLink to="/login">{{ t('auth.signIn') }}</RouterLink>
          </div>
        </template>
      </section>
    </div>
    <footer class="auth-footer">
      <span>{{ t('brand.name') }}</span
      ><span>{{ t('privacy.footer') }}</span>
    </footer>
  </main>
</template>
