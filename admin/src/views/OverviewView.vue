<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { RouterLink } from 'vue-router';

import AppIcon from '@/components/AppIcon.vue';
import StatePanel from '@/components/StatePanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import TrendChart from '@/components/TrendChart.vue';
import { apiClient } from '@/api/client';
import type { components } from '@/api/generated/schema';
import {
  formatDateTime,
  formatLatency,
  formatNumber,
  formatRate,
  shortId,
  splitBytes,
} from '@/lib/format';
import { useHealthStore } from '@/stores/health';

type AdminOverview = components['schemas']['AdminOverview'];
type AdminTrendList = components['schemas']['AdminTrendList'];
type AdminOperation = components['schemas']['AdminOperation'];
type TrendItem = AdminTrendList['items'][number];

const { t, te } = useI18n();
const health = useHealthStore();

const overview = ref<AdminOverview>();
const trends = ref<TrendItem[]>([]);
const operations = ref<AdminOperation[]>([]);
const loading = ref(true);
const error = ref<string>();
const requestId = ref<string>();
const period = ref<1 | 7 | 30>(7);

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

const mainMetrics = computed(() => {
  const value = overview.value;
  if (!value) return [];
  return [
    {
      label: 'metrics.users',
      value: formatNumber(value.users),
      unit: '',
      icon: 'users',
      extra: formatNumber(value.activeUsers),
      hint: 'metrics.activeHint',
    },
    {
      label: 'metrics.projects',
      value: formatNumber(value.projects),
      unit: '',
      icon: 'folder',
      extra: '',
      hint: 'metrics.projectsHint',
    },
    {
      label: 'metrics.devices',
      value: formatNumber(value.devices),
      unit: '',
      icon: 'monitor',
      extra: '',
      hint: 'metrics.devicesHint',
    },
    {
      label: 'metrics.storage',
      icon: 'storage',
      extra: '',
      hint: 'metrics.storageHint',
      ...splitBytes(value.payloadBytes),
    },
  ];
});

const secondaryMetrics = computed(() => {
  const value = overview.value;
  if (!value) return [];
  return [
    {
      label: 'metrics.active',
      value: formatNumber(value.activeUsers),
      queue: false,
    },
    {
      label: 'metrics.revisions',
      value: formatNumber(value.revisions),
      queue: false,
    },
    {
      label: 'metrics.tombstones',
      value: formatNumber(value.tombstones),
      queue: false,
    },
    {
      label: 'metrics.restores',
      value: formatNumber(value.queuedRestores),
      queue: value.queuedRestores > 0,
    },
  ];
});

function operationName(operation: string): string {
  return te(`operation.${operation}`) ? t(`operation.${operation}`) : operation;
}

async function load(): Promise<void> {
  loading.value = true;
  error.value = undefined;
  requestId.value = undefined;
  try {
    const [overviewData, trendData, operationData] = await Promise.all([
      apiClient.getAdminOverview(),
      apiClient.getAdminTrends(30),
      apiClient.getAdminOperations({ limit: 4, offset: 0 }),
    ]);
    overview.value = overviewData;
    trends.value = trendData.items;
    operations.value = operationData.items;
  } catch (reason) {
    error.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
    if (
      reason &&
      typeof reason === 'object' &&
      'requestId' in reason &&
      typeof reason.requestId === 'string'
    ) {
      requestId.value = reason.requestId;
    }
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section>
    <header class="page-heading">
      <div>
        <div class="eyebrow">{{ t('eyebrow.overview') }}</div>
        <h1>{{ t('title.overview') }}</h1>
        <p>{{ t('description.overview') }}</p>
      </div>
      <div class="heading-actions">
        <button class="button" :disabled="loading" @click="load">
          <AppIcon name="refresh" />{{ t('common.refresh') }}
        </button>
      </div>
    </header>

    <StatePanel
      v-if="error"
      state="error"
      :request-id="requestId"
      @retry="load"
    />
    <StatePanel v-else-if="loading" state="loading" />
    <template v-else>
      <div class="metrics-grid">
        <article
          v-for="metric in mainMetrics"
          :key="metric.label"
          class="metric-card panel"
        >
          <div class="metric-label">
            {{ t(metric.label) }}<AppIcon :name="metric.icon" />
          </div>
          <strong class="metric-number">
            {{ metric.value
            }}<small v-if="metric.unit">{{ metric.unit }}</small>
          </strong>
          <div class="metric-foot">
            <span v-if="metric.extra" class="metric-extra">{{
              metric.extra
            }}</span
            >{{ t(metric.hint) }}
          </div>
        </article>
      </div>
      <div class="secondary-metrics panel">
        <div v-for="metric in secondaryMetrics" :key="metric.label">
          <span>{{ t(metric.label) }}</span>
          <strong>{{ metric.value }}</strong>
          <span v-if="metric.queue" class="badge warning">{{
            t('metrics.queue')
          }}</span>
        </div>
      </div>

      <div class="overview-middle">
        <section class="panel trend-panel">
          <div class="panel-heading">
            <div>
              <h2>{{ t('trend.title') }}</h2>
              <p>{{ t('trend.subtitle') }}</p>
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
          <div class="trend-summary">
            <template v-if="trendStats.rate">
              <strong>{{ trendStats.rate }}<small>%</small></strong>
              <span>{{ t('trend.rate') }}</span>
              <span class="trend-total">{{
                t('trend.total', { n: formatNumber(trendStats.attempts) })
              }}</span>
            </template>
            <template v-else>
              <strong>—</strong>
              <span>{{ t('trend.rate') }}</span>
            </template>
          </div>
          <div class="chart-wrap">
            <TrendChart :points="visibleTrends" :days="period" />
          </div>
          <div class="chart-footer">
            <span><i class="legend-dot"></i>{{ t('trend.succeeded') }}</span>
            <span
              >{{ t('trend.conflicts')
              }}<b>{{ formatNumber(trendStats.conflicts) }}</b></span
            >
            <RouterLink to="/sync-attempts"
              >{{ t('trend.details') }}<AppIcon name="arrow-right"
            /></RouterLink>
          </div>
        </section>

        <section class="panel health-panel">
          <div class="panel-heading">
            <h2>{{ t('health.title') }}</h2>
            <AppIcon name="activity" />
          </div>
          <div class="health-summary">
            <div
              class="health-check"
              :class="{ failed: health.readiness === 'notReady' }"
            >
              <AppIcon
                :name="health.readiness === 'notReady' ? 'alert' : 'check'"
              />
            </div>
            <div>
              <strong>{{
                health.readiness === 'ready'
                  ? t('health.ready')
                  : health.readiness === 'notReady'
                    ? t('health.notReady')
                    : t('app.checking')
              }}</strong>
              <small v-if="health.checkedAt">{{
                t('health.checked', { time: health.checkedAt })
              }}</small>
            </div>
          </div>
          <div class="health-row">
            <span>{{ t('health.api') }}</span>
            <span :class="health.isLive ? 'text-success' : 'text-danger'"
              ><span class="small-dot"></span
              >{{
                health.isLive ? t('health.online') : t('health.disconnected')
              }}</span
            >
          </div>
          <div class="health-row">
            <span>{{ t('health.database') }}</span>
            <span :class="health.database ? 'text-success' : 'text-danger'"
              ><span class="small-dot"></span
              >{{
                health.database === null
                  ? t('health.unknown')
                  : health.database
                    ? t('health.connected')
                    : t('health.disconnected')
              }}</span
            >
          </div>
          <div class="health-row">
            <span>{{ t('health.storage') }}</span>
            <span :class="health.objectStore ? 'text-success' : 'text-danger'"
              ><span class="small-dot"></span
              >{{
                health.objectStore === null
                  ? t('health.unknown')
                  : health.objectStore
                    ? t('health.connected')
                    : t('health.disconnected')
              }}</span
            >
          </div>
          <div class="health-note">
            <AppIcon name="shield" /><span>{{ t('privacy.health') }}</span>
          </div>
        </section>
      </div>

      <section class="panel">
        <div class="panel-heading">
          <div>
            <h2>{{ t('operations.recent') }}</h2>
            <p>{{ t('operations.subtitle') }}</p>
          </div>
          <RouterLink to="/sync-attempts" class="text-link"
            >{{ t('common.viewAll') }}<AppIcon name="arrow-right"
          /></RouterLink>
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
                <th>{{ t('col.time') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in operations" :key="row.id">
                <td>
                  <span class="operation-cell"
                    ><AppIcon
                      :name="row.operation === 'restore' ? 'history' : 'sync'"
                    />{{ operationName(row.operation) }}</span
                  >
                </td>
                <td class="mono">{{ shortId(row.projectId) }}</td>
                <td>
                  <StatusBadge :value="row.status" />
                </td>
                <td class="numeric">{{ row.itemCount ?? '—' }}</td>
                <td class="numeric">{{ formatLatency(row.latencyMs) }}</td>
                <td class="muted">{{ formatDateTime(row.createdAt) }}</td>
              </tr>
              <tr v-if="!operations.length">
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
    <footer class="page-bottom">
      <span
        >{{ t('brand.name') }}<span class="footer-dot">·</span
        >{{ t('privacy.footer') }}</span
      >
    </footer>
  </section>
</template>
