<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { RouterLink, useRouter } from 'vue-router';

import AppIcon from '@/components/AppIcon.vue';
import brandIcon from '@/assets/tasktips.png';
import { useAuthStore } from '@/stores/auth';

const { t } = useI18n();
const router = useRouter();
const auth = useAuthStore();
const email = ref('');
const password = ref('');
const showPassword = ref(false);
const canSubmit = computed(
  () => email.value.trim().length > 0 && password.value.length > 0,
);

async function submit(): Promise<void> {
  if (!canSubmit.value || auth.loading) return;
  if (await auth.login(email.value, password.value)) await router.push('/');
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
        <div class="auth-illustration" aria-hidden="true">
          <div class="orbit orbit-one"></div>
          <div class="orbit orbit-two"></div>
          <div class="illustration-center">
            <img :src="brandIcon" alt="" />
          </div>
          <div class="floating-node node-desktop">
            <AppIcon name="monitor" /><span>Windows</span>
          </div>
          <div class="floating-node node-laptop">
            <AppIcon name="monitor" /><span>macOS</span>
          </div>
          <div class="floating-node node-cloud">
            <AppIcon name="cloud" /><AppIcon name="check" />
          </div>
          <i class="orbit-dot dot-one"></i><i class="orbit-dot dot-two"></i>
        </div>
        <div class="auth-trust">
          <AppIcon name="shield" /><span>{{ t('auth.trust') }}</span>
        </div>
      </section>
      <section class="auth-card panel">
        <div class="auth-card-top">
          <span class="auth-kicker">{{ t('auth.adminAccess') }}</span>
          <AppIcon name="lock" />
        </div>
        <h2>{{ t('auth.welcome') }}</h2>
        <p>{{ t('auth.loginHint') }}</p>
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
                autocomplete="current-password"
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
          <div v-if="auth.error" class="notice danger" role="alert">
            <AppIcon name="alert" />{{ auth.error }}
          </div>
          <button
            class="button primary full-width auth-submit"
            :disabled="auth.loading"
          >
            {{ t('auth.signIn') }}<AppIcon name="arrow-right" />
          </button>
        </form>
        <div class="auth-switch">
          <span>{{ t('auth.noAccount') }}</span>
          <RouterLink to="/register">{{ t('auth.register') }}</RouterLink>
        </div>
      </section>
    </div>
    <footer class="auth-footer">
      <span>{{ t('brand.name') }}</span
      ><span>{{ t('privacy.footer') }}</span>
    </footer>
  </main>
</template>
