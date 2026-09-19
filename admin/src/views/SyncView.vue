<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import StatePanel from '@/components/StatePanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { ApiError, apiClient } from '@/api/client';
import type { components } from '@/api/generated/schema';
import {
  formatDateTime,
  formatLatency,
  formatNumber,
  formatRate,
  formatTrendDay,
  shortId,
} from '@/lib/format';

type AdminOperation = components['schemas']['AdminOperation'];
type TrendItem = components['schemas']['AdminTrendList']['items'][number];

const { t, te } = useI18n();

const operations = ref<AdminOperation[]>([]);
const trends = ref<TrendItem[]>([]);
const loading = ref(true);
const error = ref<string>();
const requestId = ref<string>();
const pageOffset = ref(0);
const hasMore = ref(false);
const pageSize = 100;
const search = ref('');
const filter = ref<'all' | 'sync' | 'restore'>('all');
const period = ref<1 | 7 | 30>(7);
let loadSequence = 0;
let trendSequence = 0;
let pollTimer: ReturnType<typeof setInterval> | undefined;

const visibleTrends = computed(() => trends.value.slice(-period.value));
const trendStats = computed(() => {
  const total = visibleTrends.value.reduce(
    (sum, row) => ({
      attempts: sum.attempts + row.attempts,
      succeeded: sum.succeeded + row.succeeded,
      conflicts: sum.conflicts + row.conflicts,
    }),
    { attempts: 0, succeeded: 0, conflicts: 0 },
  );
  return { ...total, rate: formatRate(total.attempts, total.succeeded) };
});
const latest = computed(() => trends.value.at(-1));
const latestDay = computed(() =>
  latest.value ? formatTrendDay(latest.value.day) : null,
);

const filteredOperations = computed(() => {
  const query = search.value.trim().toLowerCase();
  return operations.value.filter((row) => {
    const groupMatch =
      filter.value === 'all' ||
      (filter.value === 'sync'
        ? row.operation === 'push' || row.operation === 'pull'
        : row.operation === 'restore');
    const queryMatch =
      !query ||
      (row.projectId ?? '').toLowerCase().includes(query) ||
      row.operation.toLowerCase().includes(query) ||
      (row.errorCode ?? '').toLowerCase().includes(query) ||
      row.id.toLowerCase().includes(query);
    return groupMatch && queryMatch;
  });
});

async function load(
  options: { refreshTrends?: boolean; silent?: boolean } = {},
): Promise<void> {
  const { refreshTrends = false, silent = false } = options;
  const sequence = ++loadSequence;
  // Background polls refresh data in place; only user-initiated loads show
  // the skeleton and disable the refresh button.
  if (!silent) loading.value = true;
  error.value = undefined;
  requestId.value = undefined;
  try {
    const operationsResponse = await apiClient.getAdminOperations({
      limit: pageSize,
      offset: pageOffset.value,
    });
    if (sequence !== loadSequence) return;
    operations.value = operationsResponse.items;
    hasMore.value = Boolean(operationsResponse.hasMore);
  } catch (reason) {
    if (sequence !== loadSequence) return;
    error.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
    if (reason instanceof ApiError && reason.requestId)
      requestId.value = reason.requestId;
  } finally {
    if (sequence === loadSequence && !silent) loading.value = false;
  }
  if (refreshTrends) await loadTrends();
}

async function loadTrends(): Promise<void> {
  const sequence = ++trendSequence;
  try {
    const response = await apiClient.getAdminTrends(period.value);
    if (sequence !== trendSequence) return;
    trends.value = response.items;
  } catch {
    // The operations table stays usable when trend metrics fail; summary
    // falls back to "—".
    if (sequence === trendSequence) trends.value = [];
  }
}

watch(period, () => {
  void loadTrends();
});
watch([search, filter], () => {
  pageOffset.value = 0;
});

function updatePolling(): void {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(() => {
    if (!document.hidden && !loading.value) void load({ silent: true });
  }, 15_000);
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

function operationName(operation: string): string {
  return te(`operation.${operation}`) ? t(`operation.${operation}`) : operation;
}

function statusName(status: string): string {
  return te(`status.${status}`) ? t(`status.${status}`) : status;
}

const drawerOpen = ref(false);
const drawerRow = ref<AdminOperation>();

function openDrawer(row: AdminOperation): void {
  drawerRow.value = row;
  drawerOpen.value = true;
}

function booleanLabel(value: boolean | null | undefined): string {
  if (value === null || value === undefined) return '—';
  return t(`boolean.${value}`);
}

onMounted(() => {
  updatePolling();
  void load({ refreshTrends: true });
});
onBeforeUnmount(() => {
  loadSequence += 1;
  trendSequence += 1;
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <section>
    <header class="page-heading">
      <div>
        <div class="eyebrow">{{ t('eyebrow.sync') }}</div>
        <h1>{{ t('title.sync') }}</h1>
        <p>{{ t('description.sync') }}</p>
      </div>
      <div class="heading-actions">
        <button
          class="button"
          :disabled="loading"
          @click="load({ refreshTrends: true })"
        >
          <AppIcon name="refresh" />{{ t('common.refresh') }}
        </button>
      </div>
    </header>

    <StatePanel
      v-if="error && !operations.length"
      state="error"
      :request-id="requestId"
      @retry="load({ refreshTrends: true })"
    />
    <StatePanel v-else-if="loading && !operations.length" state="loading" />
    <template v-else>
      <div class="diagnostic-summary">
        <div class="panel mini-stat">
          <span>{{ t('trend.rate') }}</span>
          <strong
            >{{ trendStats.rate ?? '—'
            }}<small v-if="trendStats.rate">%</small></strong
          >
          <span>{{ t('sync.window') }}</span>
        </div>
        <div class="panel mini-stat">
          <span>{{ t('trend.conflicts') }}</span>
          <strong>{{ formatNumber(trendStats.conflicts) }}</strong>
          <span>{{ t('sync.window') }}</span>
        </div>
        <div class="panel mini-stat">
          <span>{{ t('sync.p50') }}</span>
          <strong>
            {{ latest?.p50LatencyMs ?? '—'
            }}<small
              v-if="
                latest?.p50LatencyMs !== null &&
                latest?.p50LatencyMs !== undefined
              "
              >ms</small
            >
          </strong>
          <span v-if="latestDay">{{
            t('sync.latestDay', { day: latestDay })
          }}</span>
        </div>
        <div class="panel mini-stat">
          <span>{{ t('sync.p99') }}</span>
          <strong>
            {{ latest?.p99LatencyMs ?? '—'
            }}<small
              v-if="
                latest?.p99LatencyMs !== null &&
                latest?.p99LatencyMs !== undefined
              "
              >ms</small
            >
          </strong>
          <span v-if="latestDay">{{
            t('sync.latestDay', { day: latestDay })
          }}</span>
        </div>
      </div>

      <div v-if="error" class="notice danger" style="margin-bottom: 16px">
        <AppIcon name="alert" />{{ error }}
      </div>

      <section class="panel list-panel">
        <div class="list-tabs">
          <button
            v-for="value in ['all', 'sync', 'restore'] as const"
            :key="value"
            :class="{ active: filter === value }"
            @click="filter = value"
          >
            {{ t(`filter.${value}`) }}
          </button>
          <span class="polling"
            ><span class="small-dot"></span>{{ t('sync.polling') }}</span
          >
        </div>
        <div class="table-toolbar">
          <label class="search-field">
            <AppIcon name="search" />
            <input
              v-model="search"
              type="search"
              :placeholder="t('search.sync')"
              :aria-label="t('search.sync')"
            />
          </label>
          <span class="toolbar-note">{{ t('common.pageFilter') }}</span>
        </div>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{{ t('col.operation') }}</th>
                <th>{{ t('col.project') }}</th>
                <th>{{ t('col.status') }}</th>
                <th class="numeric">{{ t('col.items') }}</th>
                <th class="numeric">{{ t('col.latency') }}</th>
                <th>{{ t('col.errorCode') }}</th>
                <th>{{ t('col.time') }}</th>
                <th>
                  <span class="sr-only">{{ t('common.details') }}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in filteredOperations" :key="row.id">
                <td>
                  <span class="operation-cell"
                    ><AppIcon
                      :name="row.operation === 'restore' ? 'history' : 'sync'"
                    />{{ operationName(row.operation) }}</span
                  >
                  <small class="cell-sub mono">{{ shortId(row.id) }}</small>
                </td>
                <td class="mono">{{ shortId(row.projectId) }}</td>
                <td><StatusBadge :value="row.status" /></td>
                <td class="numeric">{{ row.itemCount ?? '—' }}</td>
                <td class="numeric">{{ formatLatency(row.latencyMs) }}</td>
                <td>
                  <code
                    class="error-code"
                    :class="{ 'danger-text': row.errorCode }"
                    >{{ row.errorCode || '—' }}</code
                  >
                </td>
                <td class="muted">{{ formatDateTime(row.createdAt) }}</td>
                <td>
                  <button
                    class="icon-button"
                    :aria-label="t('common.details')"
                    @click="openDrawer(row)"
                  >
                    <AppIcon name="chevron-right" />
                  </button>
                </td>
              </tr>
              <tr v-if="!filteredOperations.length">
                <td colspan="8">
                  <div class="inline-empty">
                    <AppIcon name="search" />
                    <strong>{{ t('states.noMatch') }}</strong>
                    <button
                      class="text-button"
                      @click="
                        search = '';
                        filter = 'all';
                      "
                    >
                      {{ t('common.reset') }}
                    </button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <footer class="table-footer">
          <span>{{
            t('common.pageRows', { n: filteredOperations.length })
          }}</span>
          <div class="pagination">
            <button
              class="icon-button"
              :disabled="pageOffset === 0"
              :aria-label="t('common.previous')"
              @click="previousPage"
            >
              <AppIcon name="chevron-left" />
            </button>
            <span>{{
              t('common.page', { n: pageOffset / pageSize + 1 })
            }}</span>
            <button
              class="icon-button"
              :disabled="!hasMore"
              :aria-label="t('common.next')"
              @click="nextPage"
            >
              <AppIcon name="chevron-right" />
            </button>
          </div>
        </footer>
      </section>

      <section class="panel daily-panel">
        <div class="panel-heading">
          <div>
            <h2>{{ t('sync.daily') }}</h2>
            <p>{{ t('sync.dailyHint') }}</p>
          </div>
          <div class="segmented">
            <button
              v-for="days in [1, 7, 30]"
              :key="days"
              :class="{ selected: period === days }"
              :aria-pressed="period === days"
              @click="period = days as 1 | 7 | 30"
            >
              {{ t('trend.days', { n: days }) }}
            </button>
          </div>
        </div>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{{ t('col.date') }}</th>
                <th class="numeric">{{ t('col.attempts') }}</th>
                <th class="numeric">{{ t('col.succeeded') }}</th>
                <th class="numeric">{{ t('col.conflicts') }}</th>
                <th class="numeric">{{ t('col.p50') }}</th>
                <th class="numeric">{{ t('col.p99') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="row in visibleTrends.slice(-3).reverse()"
                :key="row.day"
              >
                <td>{{ formatTrendDay(row.day) }}</td>
                <td class="numeric">{{ formatNumber(row.attempts) }}</td>
                <td class="numeric">{{ formatNumber(row.succeeded) }}</td>
                <td class="numeric">{{ row.conflicts }}</td>
                <td class="numeric">{{ row.p50LatencyMs ?? '—' }} ms</td>
                <td class="numeric">{{ row.p99LatencyMs ?? '—' }} ms</td>
              </tr>
              <tr v-if="!visibleTrends.length">
                <td colspan="6">
                  <div class="inline-empty" style="height: 120px">
                    <AppIcon name="inbox" />
                    <strong>{{ t('states.emptyTitle') }}</strong>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </template>

    <AppDialog variant="drawer" :open="drawerOpen" @close="drawerOpen = false">
      <template v-if="drawerOpen && drawerRow">
        <header class="drawer-header">
          <div>
            <span class="eyebrow">{{ t('drawer.operation') }}</span>
            <h2>{{ operationName(drawerRow.operation) }}</h2>
          </div>
          <button
            class="icon-button"
            :aria-label="t('common.close')"
            @click="drawerOpen = false"
          >
            <AppIcon name="close" />
          </button>
        </header>
        <div class="drawer-body">
          <StatusBadge :value="drawerRow.status" />
          <div class="detail-grid">
            <span>{{ t('col.id') }}</span>
            <strong class="mono">{{ drawerRow.id }}</strong>
            <span>{{ t('col.projectId') }}</span>
            <strong class="mono">{{ drawerRow.projectId ?? '—' }}</strong>
            <span>{{ t('col.deviceId') }}</span>
            <strong class="mono">{{ drawerRow.deviceId ?? '—' }}</strong>
            <span>{{ t('col.time') }}</span>
            <strong>{{ formatDateTime(drawerRow.createdAt) }}</strong>
            <span>{{ t('col.items') }}</span>
            <strong>{{ drawerRow.itemCount ?? '—' }}</strong>
            <span>{{ t('col.latency') }}</span>
            <strong>{{ formatLatency(drawerRow.latencyMs) }}</strong>
            <span>{{ t('col.errorCode') }}</span>
            <strong class="mono">{{ drawerRow.errorCode ?? '—' }}</strong>
            <template v-if="drawerRow.operation === 'restore'">
              <span>{{ t('col.retries') }}</span>
              <strong>{{ drawerRow.attempts }}</strong>
              <span>{{ t('col.runAfter') }}</span>
              <strong>{{ formatDateTime(drawerRow.runAfter) }}</strong>
              <span>{{ t('col.cancelRequested') }}</span>
              <strong>{{ booleanLabel(drawerRow.cancelRequested) }}</strong>
            </template>
          </div>
          <template v-if="drawerRow.operation === 'restore'">
            <div class="section-divider"></div>
            <h3>{{ t('restore.taskStatus') }}</h3>
            <div class="task-status">
              <AppIcon
                :name="
                  drawerRow.status === 'succeeded'
                    ? 'check'
                    : drawerRow.status === 'failed'
                      ? 'alert'
                      : 'clock'
                "
              /><span>{{ statusName(drawerRow.status) }}</span>
            </div>
            <p class="muted">{{ t('restore.taskHint') }}</p>
            <div v-if="drawerRow.status === 'failed'" class="notice warning">
              {{ t('restore.failure') }}
            </div>
          </template>
        </div>
        <footer class="drawer-footer">
          <button class="button" @click="drawerOpen = false">
            {{ t('common.close') }}
          </button>
        </footer>
      </template>
    </AppDialog>
  </section>
</template>
