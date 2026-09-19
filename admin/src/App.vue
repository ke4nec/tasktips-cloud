<script setup lang="ts">
import { storeToRefs } from 'pinia';
import { computed, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import brandIcon from '@/assets/tasktips.png';
import packageJson from '../package.json';
import { useToast } from '@/composables/useToast';
import { useAuthStore } from '@/stores/auth';
import { healthLabelKey, useHealthStore } from '@/stores/health';
import { useThemeStore } from '@/stores/theme';

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const health = useHealthStore();
const auth = useAuthStore();
// Activate the theme store so <html data-theme> is applied before first paint
// of the shell (also drives the Settings appearance section).
useThemeStore();
const { state } = storeToRefs(health);
// Release version: Docker builds inject VITE_APP_VERSION (= APP_VERSION,
// aligned with the backend Cargo version and the OpenAPI info.version);
// local dev/build falls back to admin/package.json.
// Keep in sync with Cargo.toml, contracts/openapi.yaml and deploy/*.Dockerfile.
const appVersion = import.meta.env.VITE_APP_VERSION ?? packageJson.version;

const navigation = [
  {
    label: 'nav.workspace',
    items: [
      { id: '/', icon: 'grid', page: 'overview' },
      { id: '/users', icon: 'users', page: 'users' },
      { id: '/projects', icon: 'folder', page: 'projects' },
      { id: '/devices', icon: 'monitor', page: 'devices' },
    ],
  },
  {
    label: 'nav.operations',
    items: [
      { id: '/sync-attempts', icon: 'activity', page: 'sync' },
      { id: '/audit', icon: 'audit', page: 'audit' },
    ],
  },
] as const;

const pageKey = computed(() => {
  const name = route.name;
  return name === 'sync-attempts' ? 'sync' : String(name ?? 'overview');
});

const liveState = computed(() =>
  state.value === 'live'
    ? ''
    : state.value === 'checking'
      ? 'checking'
      : 'unavailable',
);

const logoutOpen = ref(false);
const { message: toastMessage } = useToast();

onMounted(async () => {
  await auth.restore();
  await health.check();
});

watch(
  () => [auth.ready, auth.authenticated, route.path] as const,
  ([ready, authenticated, path]) => {
    if (ready && !authenticated && path !== '/login' && path !== '/register')
      void router.replace('/login');
    if (ready && authenticated && (path === '/login' || path === '/register'))
      void router.replace('/');
  },
  { immediate: true },
);

watch(
  () => route.name,
  (name) => {
    if (!name) return;
    const key = name === 'sync-attempts' ? 'sync' : String(name);
    document.title = `${t('brand.name')} · ${t(`title.${key}`)}`;
  },
);

async function signOut(): Promise<void> {
  logoutOpen.value = false;
  await auth.logout();
  await router.replace('/login');
}
</script>

<template>
  <router-view v-if="route.path === '/login' || route.path === '/register'" />
  <div v-else-if="auth.authenticated" class="app-shell">
    <aside class="sidebar">
      <span class="brand">
        <img :src="brandIcon" alt="" />
        <span
          ><strong>{{ t('brand.name') }}</strong
          ><small>{{ t('brand.console') }}</small></span
        >
      </span>
      <div class="workspace-label">
        <span class="workspace-icon"><AppIcon name="cloud" /></span>
        <span
          >{{ t('brand.workspace')
          }}<small>{{ t('brand.selfHosted') }}</small></span
        >
        <span class="small-dot"></span>
      </div>
      <nav :aria-label="t('nav.main')">
        <template v-for="group in navigation" :key="group.label">
          <p class="nav-heading">{{ t(group.label) }}</p>
          <router-link
            v-for="item in group.items"
            :key="item.id"
            :to="item.id"
            class="nav-item"
            :class="{ active: route.path === item.id }"
            :aria-current="route.path === item.id ? 'page' : undefined"
          >
            <AppIcon :name="item.icon" /><span>{{
              t(`nav.${item.page}`)
            }}</span>
          </router-link>
        </template>
      </nav>
      <div class="sidebar-bottom">
        <div class="privacy-mini">
          <AppIcon name="shield" />
          <div>
            <strong>{{ t('privacy.title') }}</strong>
            <p>{{ t('privacy.short') }}</p>
          </div>
        </div>
        <router-link
          to="/settings"
          class="nav-item"
          :class="{ active: route.path === '/settings' }"
          :aria-current="route.path === '/settings' ? 'page' : undefined"
        >
          <AppIcon name="settings" /><span>{{ t('nav.settings') }}</span>
        </router-link>
        <div class="account">
          <span class="avatar admin-avatar">A</span>
          <div>
            <strong>{{ t('brand.admin') }}</strong
            ><small>{{ t('brand.role') }}</small>
          </div>
          <button
            class="icon-button"
            :aria-label="t('common.signOut')"
            @click="logoutOpen = true"
          >
            <AppIcon name="logout" />
          </button>
        </div>
      </div>
    </aside>

    <div class="main-column">
      <header class="topbar">
        <div class="breadcrumbs">
          <span>{{ t('brand.console') }}</span
          ><span>/</span>
          <strong>{{ t(`nav.${pageKey}`) }}</strong>
        </div>
        <div class="topbar-meta">
          <span class="live-label" :class="liveState"
            ><span class="small-dot"></span>{{ t(healthLabelKey(state)) }}</span
          >
          <span class="version">v{{ appVersion }}</span>
        </div>
      </header>
      <main id="main" class="main-content">
        <router-view />
      </main>
    </div>

    <AppDialog :open="logoutOpen" @close="logoutOpen = false">
      <template v-if="logoutOpen">
        <header class="modal-header">
          <div class="modal-heading-icon"><AppIcon name="shield" /></div>
          <button
            class="icon-button"
            :aria-label="t('common.close')"
            @click="logoutOpen = false"
          >
            <AppIcon name="close" />
          </button>
        </header>
        <h2>{{ t('modal.logout') }}</h2>
        <p class="muted modal-description">{{ t('modalHint.logout') }}</p>
        <footer class="modal-footer">
          <button class="button" @click="logoutOpen = false">
            {{ t('common.cancel') }}
          </button>
          <button class="button primary" @click="signOut">
            {{ t('common.confirm') }}
          </button>
        </footer>
      </template>
    </AppDialog>
  </div>
  <main v-else class="auth-page">
    <div class="skeleton skeleton-title" style="margin-top: 20px"></div>
    <div class="skeleton"></div>
    <div class="skeleton"></div>
    <span class="sr-only">{{ t('app.checking') }}</span>
  </main>
  <div v-if="toastMessage" class="toast" role="status">
    <AppIcon name="check" />{{ toastMessage }}
  </div>
</template>
