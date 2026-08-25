<script setup lang="ts">
import { onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import { apiClient } from '@/api/client';

type Section = 'users' | 'projects' | 'devices' | 'sync' | 'audit';
type Row = Record<string, unknown>;

const props = defineProps<{ section: Section }>();
const { t } = useI18n();
const rows = ref<Row[]>([]);
const loading = ref(true);
const error = ref<string>();

const titles: Record<Section, string> = {
  users: 'navigation.users',
  projects: 'navigation.projects',
  devices: 'navigation.devices',
  sync: 'navigation.sync',
  audit: 'navigation.audit',
};

const descriptions: Record<Section, string> = {
  users: 'page.usersDescription',
  projects: 'page.projectsDescription',
  devices: 'page.devicesDescription',
  sync: 'page.syncDescription',
  audit: 'page.auditDescription',
};

const columns: Record<Section, string[]> = {
  users: ['email', 'role', 'status', 'createdAt', 'lastLoginAt'],
  projects: ['id', 'ownerUserId', 'name', 'generation', 'status', 'changeSequence'],
  devices: ['id', 'ownerUserId', 'displayName', 'platform', 'appVersion', 'revokedAt'],
  sync: ['operation', 'status', 'projectId', 'deviceId', 'itemCount', 'latencyMs', 'createdAt'],
  audit: ['action', 'actorUserId', 'subjectUserId', 'projectId', 'requestId', 'createdAt'],
};

async function load(): Promise<void> {
  loading.value = true;
  error.value = undefined;
  try {
    if (props.section === 'users') {
      rows.value = (await apiClient.getAdminUsers()).items as Row[];
    } else if (props.section === 'sync') {
      rows.value = (await apiClient.getAdminSyncAttempts()).items as Row[];
      const restores = (await apiClient.getAdminRestoreJobs()).items as Row[];
      rows.value = [...rows.value, ...restores.map((job) => ({ ...job, operation: 'restore' }))];
    } else if (props.section === 'audit') {
      rows.value = (await apiClient.getAdminAuditEvents()).items as Row[];
    } else {
      const users = (await apiClient.getAdminUsers()).items;
      const lists = await Promise.all(
        users.map((user) =>
          props.section === 'projects'
            ? apiClient.getAdminProjects(user.id)
            : apiClient.getAdminDevices(user.id),
        ),
      );
      rows.value = lists.flatMap((list) => list.items as unknown as Row[]);
    }
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : t('page.loadFailed');
  } finally {
    loading.value = false;
  }
}

function display(value: unknown): string {
  if (value === null || value === undefined || value === '') return t('table.empty');
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

watch(() => props.section, load);
onMounted(load);
</script>

<template>
  <section class="data-section">
    <header class="page-header">
      <div>
        <h1>{{ t(titles[props.section]) }}</h1>
        <p>{{ t(descriptions[props.section]) }}</p>
      </div>
      <el-button :loading="loading" @click="load">{{ t('actions.refresh') }}</el-button>
    </header>
    <el-alert v-if="error" :title="error" type="error" show-icon />
    <el-skeleton v-if="loading" :rows="5" animated />
    <el-table v-else :data="rows" stripe class="data-table" empty-text="">
      <el-table-column v-for="column in columns[props.section]" :key="column" :prop="column" :label="t(`columns.${column}`)" min-width="150">
        <template #default="scope">{{ display(scope.row[column]) }}</template>
      </el-table-column>
      <template #empty><el-empty :description="t('page.noData')" /></template>
    </el-table>
  </section>
</template>
