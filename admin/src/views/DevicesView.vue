<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import StatePanel from '@/components/StatePanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { ApiError, apiClient } from '@/api/client';
import type { components } from '@/api/generated/schema';
import { formatDateTime, shortId } from '@/lib/format';

type Device = components['schemas']['Device'];

const { t } = useI18n();

const devices = ref<Device[]>([]);
const loading = ref(true);
const error = ref<string>();
const requestId = ref<string>();
const pageOffset = ref(0);
const hasMore = ref(false);
const pageSize = 100;
const search = ref('');
const statusFilter = ref<'all' | 'active' | 'revoked'>('all');
let loadSequence = 0;

const filteredDevices = computed(() => {
  const query = search.value.trim().toLowerCase();
  return devices.value.filter((device) => {
    const status = device.revokedAt ? 'revoked' : 'active';
    const groupMatch =
      statusFilter.value === 'all' || status === statusFilter.value;
    const queryMatch =
      !query ||
      device.displayName.toLowerCase().includes(query) ||
      device.id.toLowerCase().includes(query);
    return groupMatch && queryMatch;
  });
});

async function load(): Promise<void> {
  const sequence = ++loadSequence;
  loading.value = true;
  error.value = undefined;
  requestId.value = undefined;
  try {
    const response = await apiClient.getAdminDevices({
      limit: pageSize,
      offset: pageOffset.value,
    });
    if (sequence !== loadSequence) return;
    devices.value = response.items;
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

const drawerOpen = ref(false);
const drawerDevice = ref<Device>();

function openDrawer(row: Device): void {
  drawerDevice.value = row;
  drawerOpen.value = true;
}

function deviceStatus(row: Device): string {
  return row.revokedAt ? 'revoked' : 'active';
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
        <div class="eyebrow">{{ t('eyebrow.devices') }}</div>
        <h1>{{ t('title.devices') }}</h1>
        <p>{{ t('description.devices') }}</p>
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
      <section class="panel list-panel">
        <div class="table-toolbar">
          <label class="search-field">
            <AppIcon name="search" />
            <input
              v-model="search"
              type="search"
              :placeholder="t('search.devices')"
              :aria-label="t('search.devices')"
            />
          </label>
          <label class="select-filter">
            <span class="sr-only">{{ t('col.status') }}</span>
            <select v-model="statusFilter">
              <option value="all">{{ t('filter.allStatus') }}</option>
              <option value="active">{{ t('status.active') }}</option>
              <option value="revoked">{{ t('status.revoked') }}</option>
            </select>
          </label>
          <span class="toolbar-note">{{ t('common.pageFilter') }}</span>
        </div>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{{ t('col.device') }}</th>
                <th>{{ t('col.owner') }}</th>
                <th>{{ t('col.platform') }}</th>
                <th>{{ t('col.version') }}</th>
                <th>{{ t('col.status') }}</th>
                <th>{{ t('col.lastSeen') }}</th>
                <th>
                  <span class="sr-only">{{ t('common.details') }}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in filteredDevices" :key="row.id">
                <td>
                  <button class="identity-button" @click="openDrawer(row)">
                    <span class="project-icon neutral">
                      <AppIcon name="monitor" />
                    </span>
                    <span
                      ><strong>{{ row.displayName }}</strong
                      ><small class="mono">{{ shortId(row.id) }}</small></span
                    >
                  </button>
                </td>
                <td class="mono">{{ shortId(row.ownerUserId) }}</td>
                <td>{{ row.platform }}</td>
                <td class="mono">{{ row.appVersion }}</td>
                <td><StatusBadge :value="deviceStatus(row)" /></td>
                <td class="muted">
                  {{ row.lastSeenAt ? formatDateTime(row.lastSeenAt) : '—' }}
                </td>
                <td>
                  <button class="text-button" @click="openDrawer(row)">
                    {{ t('common.details') }}
                  </button>
                </td>
              </tr>
              <tr v-if="!filteredDevices.length">
                <td colspan="7">
                  <div class="inline-empty">
                    <AppIcon name="search" />
                    <strong>{{ t('states.noMatch') }}</strong>
                    <button
                      class="text-button"
                      @click="
                        search = '';
                        statusFilter = 'all';
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
          <span>{{ t('common.pageRows', { n: filteredDevices.length }) }}</span>
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
    </template>

    <AppDialog variant="drawer" :open="drawerOpen" @close="drawerOpen = false">
      <template v-if="drawerOpen && drawerDevice">
        <header class="drawer-header">
          <div>
            <span class="eyebrow">{{ t('drawer.device') }}</span>
            <h2>{{ drawerDevice.displayName }}</h2>
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
          <StatusBadge :value="deviceStatus(drawerDevice)" />
          <div class="detail-grid">
            <span>{{ t('col.id') }}</span>
            <strong class="mono">{{ drawerDevice.id }}</strong>
            <span>{{ t('col.userId') }}</span>
            <strong class="mono">{{ drawerDevice.ownerUserId }}</strong>
            <span>{{ t('col.platform') }}</span>
            <strong>{{ drawerDevice.platform }}</strong>
            <span>{{ t('col.version') }}</span>
            <strong class="mono">{{ drawerDevice.appVersion }}</strong>
            <span>{{ t('col.created') }}</span>
            <strong>{{ formatDateTime(drawerDevice.createdAt) }}</strong>
            <span>{{ t('col.lastSeen') }}</span>
            <strong>{{ formatDateTime(drawerDevice.lastSeenAt) }}</strong>
            <span>{{ t('col.lastPull') }}</span>
            <strong>{{ formatDateTime(drawerDevice.lastPullAt) }}</strong>
            <span>{{ t('col.lastPush') }}</span>
            <strong>{{ formatDateTime(drawerDevice.lastPushAt) }}</strong>
            <span>{{ t('col.revokedAt') }}</span>
            <strong>{{ formatDateTime(drawerDevice.revokedAt) }}</strong>
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
