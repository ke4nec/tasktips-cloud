<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import { apiClient } from '@/api/client';

type Section = 'users' | 'projects' | 'devices' | 'sync' | 'audit';
type Row = Record<string, unknown>;

const props = defineProps<{ section: Section }>();
const { t } = useI18n();
const rows = ref<Row[]>([]);
const loading = ref(true);
const error = ref<string>();
const restoreProjectId = ref('');
const restoreSequence = ref<number | undefined>();
const restoreReason = ref('');
const restoreLoading = ref(false);
const restoreMessage = ref('');
const pageOffset = ref(0);
const hasMore = ref(false);
const pageSize = 100;
const trendRows = ref<Row[]>([]);
let refreshTimer: ReturnType<typeof setInterval> | undefined;

const projectRows = computed(() =>
  props.section === 'projects' ? rows.value : [],
);

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
  projects: [
    'id',
    'ownerUserId',
    'name',
    'generation',
    'status',
    'changeSequence',
  ],
  devices: [
    'id',
    'ownerUserId',
    'displayName',
    'platform',
    'appVersion',
    'revokedAt',
  ],
  sync: [
    'operation',
    'status',
    'attempts',
    'runAfter',
    'cancelRequested',
    'projectId',
    'deviceId',
    'itemCount',
    'latencyMs',
    'createdAt',
  ],
  audit: [
    'action',
    'actorUserId',
    'subjectUserId',
    'projectId',
    'requestId',
    'createdAt',
  ],
};

async function load(reset = false): Promise<void> {
  if (reset) pageOffset.value = 0;
  loading.value = true;
  error.value = undefined;
  trendRows.value = [];
  try {
    const page = { limit: pageSize, offset: pageOffset.value };
    if (props.section === 'users') {
      const response = await apiClient.getAdminUsers(page);
      rows.value = response.items as Row[];
      hasMore.value = Boolean(response.hasMore);
    } else if (props.section === 'sync') {
      const [attempts, restoreJobs, jobs] = await Promise.all([
        apiClient.getAdminSyncAttempts(page),
        apiClient.getAdminRestoreJobs(page),
        apiClient.getAdminJobs(page),
      ]);
      rows.value = attempts.items as Row[];
      const restores = restoreJobs.items as Row[];
      rows.value = [
        ...rows.value,
        ...restores.map((job) => ({ ...job, operation: 'restore' })),
        ...(jobs.items as Row[]).map((job) => ({
          ...job,
          operation: job.kind,
        })),
      ];
      hasMore.value = Boolean(
        attempts.hasMore || restoreJobs.hasMore || jobs.hasMore,
      );
      const trends = await apiClient.getAdminTrends();
      trendRows.value = trends.items as Row[];
    } else if (props.section === 'audit') {
      const response = await apiClient.getAdminAuditEvents(page);
      rows.value = response.items as Row[];
      hasMore.value = Boolean(response.hasMore);
    } else {
      const users = (await apiClient.getAdminUsers(page)).items;
      const lists = await Promise.all(
        users.map((user) =>
          props.section === 'projects'
            ? apiClient.getAdminProjects(user.id, page)
            : apiClient.getAdminDevices(user.id, page),
        ),
      );
      rows.value = lists.flatMap((list) => list.items as unknown as Row[]);
      hasMore.value = Boolean(lists.some((list) => list.hasMore));
    }
    if (
      !projectRows.value.some(
        (project) => project.id === restoreProjectId.value,
      )
    ) {
      restoreProjectId.value = '';
    }
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('page.loadFailed');
  } finally {
    loading.value = false;
  }
}

function nextPage(): void {
  if (!hasMore.value) return;
  pageOffset.value += pageSize;
  void load();
}

function previousPage(): void {
  pageOffset.value = Math.max(0, pageOffset.value - pageSize);
  void load();
}

function updatePolling(): void {
  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = undefined;
  if (props.section === 'sync') {
    refreshTimer = setInterval(() => {
      if (!loading.value) void load();
    }, 15_000);
  }
}

async function queueRestore(): Promise<void> {
  restoreMessage.value = '';
  error.value = undefined;
  if (
    !restoreProjectId.value ||
    restoreSequence.value === undefined ||
    !restoreReason.value.trim()
  ) {
    error.value = t('restore.required');
    return;
  }
  restoreLoading.value = true;
  try {
    await apiClient.createAdminRestore(restoreProjectId.value, {
      targetChangeSequence: restoreSequence.value,
      reason: restoreReason.value.trim(),
    });
    restoreMessage.value = t('restore.queued');
    restoreSequence.value = undefined;
    restoreReason.value = '';
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('page.loadFailed');
  } finally {
    restoreLoading.value = false;
  }
}

function display(value: unknown): string {
  if (value === null || value === undefined || value === '')
    return t('table.empty');
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

watch(
  () => props.section,
  () => {
    updatePolling();
    void load(true);
  },
);
onMounted(() => {
  updatePolling();
  void load(true);
});
onBeforeUnmount(() => {
  if (refreshTimer) clearInterval(refreshTimer);
});
</script>

<template>
  <section class="data-section">
    <header class="page-header">
      <div>
        <h1>{{ t(titles[props.section]) }}</h1>
        <p>{{ t(descriptions[props.section]) }}</p>
      </div>
      <el-button :loading="loading" @click="load(true)">{{
        t('actions.refresh')
      }}</el-button>
    </header>
    <el-alert v-if="error" :title="error" type="error" show-icon />
    <el-alert
      v-if="restoreMessage"
      :title="restoreMessage"
      type="success"
      show-icon
    />
    <el-form
      v-if="props.section === 'projects' && !loading"
      class="restore-form"
      inline
    >
      <el-form-item :label="t('restore.project')">
        <el-select
          v-model="restoreProjectId"
          :placeholder="t('restore.projectPlaceholder')"
          filterable
        >
          <el-option
            v-for="project in projectRows"
            :key="String(project.id)"
            :label="`${display(project.name)} (${display(project.id)})`"
            :value="String(project.id)"
          />
        </el-select>
      </el-form-item>
      <el-form-item :label="t('restore.sequence')">
        <el-input-number v-model="restoreSequence" :min="0" :precision="0" />
      </el-form-item>
      <el-form-item :label="t('restore.reason')">
        <el-input
          v-model="restoreReason"
          :placeholder="t('restore.reasonPlaceholder')"
          maxlength="500"
        />
      </el-form-item>
      <el-form-item>
        <el-button
          type="primary"
          :loading="restoreLoading"
          @click="queueRestore"
        >
          {{ t('restore.submit') }}
        </el-button>
      </el-form-item>
    </el-form>
    <el-skeleton v-if="loading" :rows="5" animated />
    <el-table v-else :data="rows" stripe class="data-table" empty-text="">
      <el-table-column
        v-for="column in columns[props.section]"
        :key="column"
        :prop="column"
        :label="t(`columns.${column}`)"
        min-width="150"
      >
        <template #default="scope">{{ display(scope.row[column]) }}</template>
      </el-table-column>
      <template #empty><el-empty :description="t('page.noData')" /></template>
    </el-table>
    <div
      v-if="props.section === 'sync' && trendRows.length"
      class="trend-table"
    >
      <h2>{{ t('trend.title') }}</h2>
      <el-table :data="trendRows" stripe class="data-table" empty-text="">
        <el-table-column prop="day" :label="t('trend.day')" min-width="160" />
        <el-table-column
          prop="attempts"
          :label="t('trend.attempts')"
          min-width="120"
        />
        <el-table-column
          prop="succeeded"
          :label="t('trend.succeeded')"
          min-width="120"
        />
        <el-table-column
          prop="conflicts"
          :label="t('trend.conflicts')"
          min-width="120"
        />
        <el-table-column
          prop="p50LatencyMs"
          :label="t('trend.p50')"
          min-width="120"
        />
        <el-table-column
          prop="p99LatencyMs"
          :label="t('trend.p99')"
          min-width="120"
        />
      </el-table>
    </div>
    <div v-if="!loading && (rows.length || hasMore)" class="page-controls">
      <el-button :disabled="pageOffset === 0" @click="previousPage">
        {{ t('pagination.previous') }}
      </el-button>
      <span>{{
        t('pagination.page', { page: pageOffset / pageSize + 1 })
      }}</span>
      <el-button :disabled="!hasMore" @click="nextPage">
        {{ t('pagination.next') }}
      </el-button>
    </div>
  </section>
</template>
