<script setup lang="ts">
import { ElAlert } from 'element-plus/es/components/alert/index.mjs';
import 'element-plus/es/components/alert/style/css.mjs';
import { ElButton } from 'element-plus/es/components/button/index.mjs';
import 'element-plus/es/components/button/style/css.mjs';
import { ElEmpty } from 'element-plus/es/components/empty/index.mjs';
import 'element-plus/es/components/empty/style/css.mjs';
import { ElForm, ElFormItem } from 'element-plus/es/components/form/index.mjs';
import 'element-plus/es/components/form/style/css.mjs';
import 'element-plus/es/components/form-item/style/css.mjs';
import { ElInput } from 'element-plus/es/components/input/index.mjs';
import 'element-plus/es/components/input/style/css.mjs';
import { ElInputNumber } from 'element-plus/es/components/input-number/index.mjs';
import 'element-plus/es/components/input-number/style/css.mjs';
import {
  ElSelect,
  ElOption,
} from 'element-plus/es/components/select/index.mjs';
import 'element-plus/es/components/select/style/css.mjs';
import { ElSkeleton } from 'element-plus/es/components/skeleton/index.mjs';
import 'element-plus/es/components/skeleton/style/css.mjs';
import {
  ElTable,
  ElTableColumn,
} from 'element-plus/es/components/table/index.mjs';
import 'element-plus/es/components/table/style/css.mjs';
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
const restoreProjectRows = ref<Row[]>([]);
const projectSearchLoading = ref(false);
let refreshTimer: ReturnType<typeof setInterval> | undefined;
let loadSequence = 0;
let projectSearchSequence = 0;

const projectRows = computed(() => {
  if (props.section !== 'projects') return [];
  const projects = new Map<string, Row>();
  for (const project of [...rows.value, ...restoreProjectRows.value]) {
    const id = project.id;
    if (id !== undefined && id !== null) projects.set(String(id), project);
  }
  return [...projects.values()];
});

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

async function load(reset = false, refreshTrends = true): Promise<void> {
  if (reset) pageOffset.value = 0;
  const sequence = ++loadSequence;
  const section = props.section;
  loading.value = true;
  error.value = undefined;
  if (section === 'sync' && refreshTrends) trendRows.value = [];
  try {
    const page = { limit: pageSize, offset: pageOffset.value };
    if (section === 'users') {
      const response = await apiClient.getAdminUsers(page);
      if (sequence !== loadSequence) return;
      rows.value = response.items as Row[];
      hasMore.value = Boolean(response.hasMore);
    } else if (section === 'sync') {
      const [operations, trends] = await Promise.all([
        apiClient.getAdminOperations(page),
        refreshTrends ? apiClient.getAdminTrends() : Promise.resolve(undefined),
      ]);
      if (sequence !== loadSequence) return;
      rows.value = operations.items as Row[];
      hasMore.value = Boolean(operations.hasMore);
      if (trends) trendRows.value = trends.items as Row[];
    } else if (section === 'audit') {
      const response = await apiClient.getAdminAuditEvents(page);
      if (sequence !== loadSequence) return;
      rows.value = response.items as Row[];
      hasMore.value = Boolean(response.hasMore);
    } else if (section === 'projects') {
      const response = await apiClient.getAdminProjects(page);
      if (sequence !== loadSequence) return;
      rows.value = response.items as Row[];
      restoreProjectRows.value = mergeRows(
        restoreProjectRows.value,
        response.items as Row[],
      );
      hasMore.value = Boolean(response.hasMore);
    } else {
      const response = await apiClient.getAdminDevices(page);
      if (sequence !== loadSequence) return;
      rows.value = response.items as Row[];
      hasMore.value = Boolean(response.hasMore);
    }
  } catch (reason) {
    if (sequence === loadSequence) {
      error.value =
        reason instanceof Error ? reason.message : t('page.loadFailed');
    }
  } finally {
    if (sequence === loadSequence) loading.value = false;
  }
}

async function searchProjects(query: string): Promise<void> {
  if (props.section !== 'projects') return;
  const sequence = ++projectSearchSequence;
  projectSearchLoading.value = true;
  try {
    const response = await apiClient.getAdminProjects({
      search: query.trim() || undefined,
      limit: 50,
      offset: 0,
    });
    if (sequence === projectSearchSequence && props.section === 'projects') {
      restoreProjectRows.value = mergeRows(
        restoreProjectRows.value,
        response.items as Row[],
      );
    }
  } catch (reason) {
    if (sequence === projectSearchSequence) {
      error.value =
        reason instanceof Error ? reason.message : t('page.loadFailed');
    }
  } finally {
    if (sequence === projectSearchSequence) projectSearchLoading.value = false;
  }
}

function mergeRows(existing: Row[], additions: Row[]): Row[] {
  const rowsById = new Map<string, Row>();
  for (const row of [...existing, ...additions]) {
    const id = row.id;
    if (id !== undefined && id !== null) rowsById.set(String(id), row);
  }
  return [...rowsById.values()];
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
      if (!loading.value && !document.hidden) void load(false, false);
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
    projectSearchSequence += 1;
    updatePolling();
    void load(true, true);
  },
);
onMounted(() => {
  updatePolling();
  void load(true, true);
});
onBeforeUnmount(() => {
  loadSequence += 1;
  projectSearchSequence += 1;
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
          remote
          :remote-method="searchProjects"
          :loading="projectSearchLoading"
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
