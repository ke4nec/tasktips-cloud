<script setup lang="ts">
import { ElAlert } from 'element-plus/es/components/alert/index.mjs';
import 'element-plus/es/components/alert/style/css.mjs';
import { ElButton } from 'element-plus/es/components/button/index.mjs';
import 'element-plus/es/components/button/style/css.mjs';
import { ElCard } from 'element-plus/es/components/card/index.mjs';
import 'element-plus/es/components/card/style/css.mjs';
import { ElSkeleton } from 'element-plus/es/components/skeleton/index.mjs';
import 'element-plus/es/components/skeleton/style/css.mjs';
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';

import { apiClient } from '@/api/client';
import { useAuthStore } from '@/stores/auth';

const { t } = useI18n();
const auth = useAuthStore();
const overview = ref<Awaited<ReturnType<typeof apiClient.getAdminOverview>>>();
const error = ref<string>();
const loading = ref(true);
const cards = computed(() => {
  const value = overview.value;
  if (!value) return [];
  return [
    { label: t('overview.users'), value: value.users },
    { label: t('overview.activeUsers'), value: value.activeUsers },
    { label: t('overview.projects'), value: value.projects },
    { label: t('overview.devices'), value: value.devices },
    { label: t('overview.revisions'), value: value.revisions },
    { label: t('overview.tombstones'), value: value.tombstones },
    {
      label: t('overview.payloadBytes'),
      value: formatBytes(value.payloadBytes),
    },
    { label: t('overview.queuedRestores'), value: value.queuedRestores },
  ];
});

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  if (value < 1024 * 1024 * 1024)
    return `${(value / 1024 / 1024).toFixed(1)} MiB`;
  return `${(value / 1024 / 1024 / 1024).toFixed(1)} GiB`;
}

async function load(): Promise<void> {
  loading.value = true;
  error.value = undefined;
  try {
    overview.value = await apiClient.getAdminOverview();
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('page.loadFailed');
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section class="data-section">
    <header class="page-header">
      <div>
        <h1>{{ t('navigation.overview') }}</h1>
        <p>{{ t('page.overviewDescription') }}</p>
      </div>
      <el-button :loading="loading" @click="load">{{
        t('actions.refresh')
      }}</el-button>
    </header>
    <el-alert v-if="error" :title="error" type="error" show-icon />
    <el-skeleton v-if="loading" :rows="4" animated />
    <div v-else class="metric-grid">
      <el-card
        v-for="card in cards"
        :key="card.label"
        shadow="never"
        class="metric-card"
      >
        <span>{{ card.label }}</span>
        <strong>{{ card.value }}</strong>
      </el-card>
    </div>
    <el-alert
      class="privacy-note"
      :title="t('overview.privacyNote')"
      type="info"
      :closable="false"
    />
    <el-button link type="primary" @click="auth.logout">{{
      t('auth.signOut')
    }}</el-button>
  </section>
</template>
