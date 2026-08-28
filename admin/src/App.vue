<script setup lang="ts">
import {
  Connection,
  DataAnalysis,
  Files,
  Monitor,
  Tickets,
  User,
} from '@element-plus/icons-vue';
import {
  ElAside,
  ElContainer,
  ElHeader,
  ElMain,
} from 'element-plus/es/components/container/index.mjs';
import 'element-plus/es/components/container/style/css.mjs';
import { ElIcon } from 'element-plus/es/components/icon/index.mjs';
import 'element-plus/es/components/icon/style/css.mjs';
import { ElMenu, ElMenuItem } from 'element-plus/es/components/menu/index.mjs';
import 'element-plus/es/components/menu/style/css.mjs';
import { ElSkeleton } from 'element-plus/es/components/skeleton/index.mjs';
import 'element-plus/es/components/skeleton/style/css.mjs';
import { ElTag } from 'element-plus/es/components/tag/index.mjs';
import 'element-plus/es/components/tag/style/css.mjs';
import { storeToRefs } from 'pinia';
import { onMounted, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';

import { healthLabelKey, useHealthStore } from '@/stores/health';
import { useAuthStore } from '@/stores/auth';

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const health = useHealthStore();
const auth = useAuthStore();
const { state, isLive } = storeToRefs(health);

onMounted(async () => {
  await auth.restore();
  await health.check();
});

watch(
  () => [auth.ready, auth.authenticated, route.path] as const,
  ([ready, authenticated, path]) => {
    if (ready && !authenticated && path !== '/login')
      void router.replace('/login');
    if (ready && authenticated && path === '/login') void router.replace('/');
  },
  { immediate: true },
);
</script>

<template>
  <router-view v-if="route.path === '/login'" />
  <el-container v-else-if="auth.authenticated" class="app-shell">
    <el-header class="topbar">
      <strong>{{ t('app.name') }}</strong>
      <el-tag :type="isLive ? 'success' : 'info'" effect="plain" size="small">
        {{ t(healthLabelKey(state)) }}
      </el-tag>
    </el-header>

    <el-container class="workspace">
      <el-aside class="sidebar" width="208px">
        <el-menu :default-active="route.path" router>
          <el-menu-item index="/">
            <el-icon><DataAnalysis /></el-icon>
            <span>{{ t('navigation.overview') }}</span>
          </el-menu-item>
          <el-menu-item index="/users">
            <el-icon><User /></el-icon>
            <span>{{ t('navigation.users') }}</span>
          </el-menu-item>
          <el-menu-item index="/projects">
            <el-icon><Files /></el-icon>
            <span>{{ t('navigation.projects') }}</span>
          </el-menu-item>
          <el-menu-item index="/devices">
            <el-icon><Monitor /></el-icon>
            <span>{{ t('navigation.devices') }}</span>
          </el-menu-item>
          <el-menu-item index="/sync-attempts">
            <el-icon><Connection /></el-icon>
            <span>{{ t('navigation.sync') }}</span>
          </el-menu-item>
          <el-menu-item index="/audit">
            <el-icon><Tickets /></el-icon>
            <span>{{ t('navigation.audit') }}</span>
          </el-menu-item>
        </el-menu>
      </el-aside>

      <el-main class="main-content">
        <router-view />
      </el-main>
    </el-container>
  </el-container>
  <main v-else class="auth-loading"><el-skeleton :rows="3" animated /></main>
</template>
