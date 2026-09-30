<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import StatePanel from '@/components/StatePanel.vue';
import { ApiError, apiClient } from '@/api/client';
import type { components } from '@/api/generated/schema';
import { auditActionKey, auditMetadataRows } from '@/lib/audit';
import { formatDateTime, shortId } from '@/lib/format';
import { useToast } from '@/composables/useToast';

type AuditEvent = components['schemas']['AuditEvent'];

const { t } = useI18n();
const toast = useToast();

const events = ref<AuditEvent[]>([]);
const loading = ref(true);
const error = ref<string>();
const requestId = ref<string>();
const pageOffset = ref(0);
const hasMore = ref(false);
const pageSize = 100;
const search = ref('');
const actionFilter = ref('');
const exporting = ref(false);
let loadSequence = 0;

const AUDIT_FILTER_OPTIONS = [
  { value: '', labelKey: 'audit.filterAll' },
  { value: 'auth.login_failed', labelKey: 'auditAction.auth.loginFailed' },
  { value: 'auth.refresh_failed', labelKey: 'auditAction.auth.refreshFailed' },
  { value: 'auth.reauth_failed', labelKey: 'auditAction.auth.reauthFailed' },
  { value: 'auth.login', labelKey: 'auditAction.auth.login' },
  { value: 'auth.refresh', labelKey: 'auditAction.auth.refresh' },
];

const filteredEvents = computed(() => {
  const query = search.value.trim().toLowerCase();
  if (!query) return events.value;
  return events.value.filter((event) => {
    const label = auditActionKey(event.action)?.toLowerCase() ?? '';
    const translated = t(`auditAction.${label}`).toLowerCase();
    return (
      event.action.toLowerCase().includes(query) ||
      translated.includes(query) ||
      (event.requestId ?? '').toLowerCase().includes(query) ||
      (event.actorUserId ?? '').toLowerCase().includes(query) ||
      (event.subjectUserId ?? '').toLowerCase().includes(query)
    );
  });
});

function actionLabel(event: AuditEvent): string {
  const key = auditActionKey(event.action);
  return key ? t(`auditAction.${key}`) : event.action;
}

async function load(): Promise<void> {
  const sequence = ++loadSequence;
  loading.value = true;
  error.value = undefined;
  requestId.value = undefined;
  try {
    const response = await apiClient.getAdminAuditEvents({
      limit: pageSize,
      offset: pageOffset.value,
      action: actionFilter.value || undefined,
    });
    if (sequence !== loadSequence) return;
    events.value = response.items;
    hasMore.value = Boolean(response.hasMore);
  } catch (reason) {
    if (sequence !== loadSequence) return;
    error.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
    if (reason instanceof ApiError && reason.requestId)
      requestId.value = reason.requestId;
  } finally {
    if (sequence === loadSequence) loading.value = false;
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
function onActionFilterChange(): void {
  pageOffset.value = 0;
  void load();
}

async function exportCsv(): Promise<void> {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const csv = await apiClient.exportAuditCsv(actionFilter.value || undefined);
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = url;
    link.download = 'tasktips-audit.csv';
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    toast.show(t('audit.exported'));
  } catch {
    toast.show(t('audit.exportFailed'));
  } finally {
    exporting.value = false;
  }
}

const drawerOpen = ref(false);
const drawerEvent = ref<AuditEvent>();

function openDrawer(event: AuditEvent): void {
  drawerEvent.value = event;
  drawerOpen.value = true;
}

onMounted(load);
onBeforeUnmount(() => {
  loadSequence += 1;
});
</script>

<template>
  <section>
    <header class="page-heading">
      <div>
        <div class="eyebrow">{{ t('eyebrow.audit') }}</div>
        <h1>{{ t('title.audit') }}</h1>
        <p>{{ t('description.audit') }}</p>
      </div>
      <div class="heading-actions">
        <button class="button" :disabled="exporting" @click="exportCsv">
          <AppIcon name="download" />{{ t('audit.export') }}
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
      <section class="panel list-panel">
        <div class="table-toolbar">
          <label class="search-field">
            <AppIcon name="search" />
            <input
              v-model="search"
              type="search"
              :placeholder="t('search.audit')"
              :aria-label="t('search.audit')"
            />
          </label>
          <label class="filter-field">
            <span class="sr-only">{{ t('audit.filterAction') }}</span>
            <select
              v-model="actionFilter"
              :aria-label="t('audit.filterAction')"
              @change="onActionFilterChange"
            >
              <option
                v-for="option in AUDIT_FILTER_OPTIONS"
                :key="option.value"
                :value="option.value"
              >
                {{ t(option.labelKey) }}
              </option>
            </select>
          </label>
          <span class="toolbar-note">{{ t('common.pageFilter') }}</span>
        </div>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{{ t('col.event') }}</th>
                <th>{{ t('col.actor') }}</th>
                <th>{{ t('col.subject') }}</th>
                <th>{{ t('col.request') }}</th>
                <th>{{ t('col.time') }}</th>
                <th>
                  <span class="sr-only">{{ t('common.details') }}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in filteredEvents" :key="row.id">
                <td>
                  <span class="event-title">{{ actionLabel(row) }}</span>
                  <small class="cell-sub mono">{{ row.action }}</small>
                </td>
                <td class="mono">{{ shortId(row.actorUserId) }}</td>
                <td class="mono">{{ shortId(row.subjectUserId) }}</td>
                <td class="mono">{{ row.requestId ?? '—' }}</td>
                <td class="muted">{{ formatDateTime(row.createdAt) }}</td>
                <td>
                  <button class="text-button" @click="openDrawer(row)">
                    {{ t('common.details') }}
                  </button>
                </td>
              </tr>
              <tr v-if="!filteredEvents.length">
                <td colspan="6">
                  <div class="inline-empty">
                    <AppIcon name="search" />
                    <strong>{{ t('states.noMatch') }}</strong>
                    <button class="text-button" @click="search = ''">
                      {{ t('common.reset') }}
                    </button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <footer class="table-footer">
          <span>{{ t('common.pageRows', { n: filteredEvents.length }) }}</span>
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
      <div class="page-note">
        <AppIcon name="shield" /><span>{{ t('audit.note') }}</span>
      </div>
    </template>

    <AppDialog variant="drawer" :open="drawerOpen" @close="drawerOpen = false">
      <template v-if="drawerOpen && drawerEvent">
        <header class="drawer-header">
          <div>
            <span class="eyebrow">{{ t('drawer.audit') }}</span>
            <h2>{{ actionLabel(drawerEvent) }}</h2>
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
          <code class="mono">{{ drawerEvent.action }}</code>
          <div class="detail-grid">
            <span>{{ t('col.id') }}</span>
            <strong>{{ drawerEvent.id }}</strong>
            <span>{{ t('col.actor') }}</span>
            <strong class="mono">{{ drawerEvent.actorUserId ?? '—' }}</strong>
            <span>{{ t('col.subject') }}</span>
            <strong class="mono">{{ drawerEvent.subjectUserId ?? '—' }}</strong>
            <span>{{ t('col.projectId') }}</span>
            <strong class="mono">{{ drawerEvent.projectId ?? '—' }}</strong>
            <span>{{ t('col.request') }}</span>
            <strong class="mono">{{ drawerEvent.requestId ?? '—' }}</strong>
            <span>{{ t('col.time') }}</span>
            <strong>{{ formatDateTime(drawerEvent.createdAt) }}</strong>
            <template
              v-for="row in auditMetadataRows(drawerEvent.metadata)"
              :key="row.label"
            >
              <span>{{ t(row.label) }}</span>
              <strong class="mono">{{ row.value }}</strong>
            </template>
          </div>
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

<style scoped>
.event-title {
  display: block;
  font-weight: 500;
}
</style>
