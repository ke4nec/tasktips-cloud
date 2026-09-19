<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRouter } from 'vue-router';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import StatePanel from '@/components/StatePanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { ApiError, apiClient } from '@/api/client';
import type { components } from '@/api/generated/schema';
import { formatDateTime, formatNumber, shortId } from '@/lib/format';
import { useToast } from '@/composables/useToast';

type Project = components['schemas']['Project'];
type AdminHistoryItem = components['schemas']['AdminHistoryItem'];

const { t } = useI18n();
const router = useRouter();
const toast = useToast();

const projects = ref<Project[]>([]);
const loading = ref(true);
const error = ref<string>();
const requestId = ref<string>();
const pageOffset = ref(0);
const hasMore = ref(false);
const pageSize = 100;
const search = ref('');
const statusFilter = ref<
  'all' | 'active' | 'maintenance' | 'disabled' | 'deleting'
>('all');
let loadSequence = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;

const filteredProjects = computed(() =>
  projects.value.filter(
    (project) =>
      statusFilter.value === 'all' || project.status === statusFilter.value,
  ),
);

async function load(): Promise<void> {
  const sequence = ++loadSequence;
  loading.value = true;
  error.value = undefined;
  requestId.value = undefined;
  try {
    const response = await apiClient.getAdminProjects({
      search: search.value.trim() || undefined,
      limit: pageSize,
      offset: pageOffset.value,
    });
    if (sequence !== loadSequence) return;
    projects.value = response.items;
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

watch(search, () => {
  if (searchTimer) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    pageOffset.value = 0;
    void load();
  }, 300);
});

watch(statusFilter, () => {
  pageOffset.value = 0;
});

onBeforeUnmount(() => {
  loadSequence += 1;
  if (searchTimer) clearTimeout(searchTimer);
});

function nextPage(): void {
  if (!hasMore.value) return;
  pageOffset.value += pageSize;
  void load();
}
function previousPage(): void {
  pageOffset.value = Math.max(0, pageOffset.value - pageSize);
  void load();
}

/* Detail drawer with history metadata */
const drawerOpen = ref(false);
const drawerProject = ref<Project>();
const history = ref<AdminHistoryItem[]>([]);
const historyLoading = ref(false);
const historyError = ref(false);
let historySequence = 0;

async function openDrawer(row: Project): Promise<void> {
  drawerProject.value = row;
  history.value = [];
  historyError.value = false;
  drawerOpen.value = true;
  const sequence = ++historySequence;
  historyLoading.value = true;
  try {
    // The history endpoint paginates by afterSequence, not offset.
    const response = await apiClient.getAdminProjectHistory(row.id, {
      limit: 20,
    });
    if (sequence !== historySequence) return;
    history.value = response.items;
  } catch {
    if (sequence === historySequence) historyError.value = true;
  } finally {
    if (sequence === historySequence) historyLoading.value = false;
  }
}

function historyTitle(item: AdminHistoryItem): string {
  return item.tombstone ? t('projects.tombstone') : t('projects.revision');
}

/* Two-step restore */
const restoreOpen = ref(false);
const restoreStep = ref<1 | 2>(1);
const restoreLoading = ref(false);
const restoreError = ref('');
const targetType = ref<'sequence' | 'snapshot'>('sequence');
const formSequence = ref('');
const formSnapshotId = ref('');
const formReason = ref('');
const formConfirmId = ref('');

function startRestore(row: Project): void {
  if (row.status !== 'active') return;
  drawerOpen.value = false;
  targetType.value = 'sequence';
  formSequence.value = String(row.changeSequence);
  formSnapshotId.value = '';
  formReason.value = '';
  formConfirmId.value = '';
  restoreError.value = '';
  restoreStep.value = 1;
  restoreOpen.value = true;
}

function backStep(): void {
  if (restoreStep.value === 2) restoreStep.value = 1;
  else restoreOpen.value = false;
}

async function submitRestore(): Promise<void> {
  const project = drawerProject.value;
  if (!project || restoreLoading.value) return;
  restoreError.value = '';
  if (!formReason.value.trim()) {
    restoreError.value = t('common.required');
    return;
  }
  if (targetType.value === 'sequence') {
    const sequence = Number(formSequence.value);
    if (
      formSequence.value === '' ||
      !Number.isSafeInteger(sequence) ||
      sequence < 0 ||
      sequence > project.changeSequence
    ) {
      restoreError.value = t('restore.invalidSequence');
      return;
    }
  }
  if (restoreStep.value === 1) {
    restoreStep.value = 2;
    return;
  }
  if (formConfirmId.value.trim() !== project.id) {
    restoreError.value = t('common.idMismatch');
    return;
  }
  restoreLoading.value = true;
  try {
    await apiClient.createAdminRestore(
      project.id,
      targetType.value === 'sequence'
        ? {
            targetChangeSequence: Number(formSequence.value),
            reason: formReason.value.trim(),
          }
        : {
            snapshotId: formSnapshotId.value.trim(),
            reason: formReason.value.trim(),
          },
    );
    restoreOpen.value = false;
    toast.show(t('restore.queued'));
    await router.push('/sync-attempts');
  } catch (reason) {
    restoreError.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
  } finally {
    restoreLoading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <section>
    <header class="page-heading">
      <div>
        <div class="eyebrow">{{ t('eyebrow.projects') }}</div>
        <h1>{{ t('title.projects') }}</h1>
        <p>{{ t('description.projects') }}</p>
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
              :placeholder="t('search.projects')"
              :aria-label="t('search.projects')"
            />
          </label>
          <label class="select-filter">
            <span class="sr-only">{{ t('col.status') }}</span>
            <select v-model="statusFilter">
              <option value="all">{{ t('filter.allStatus') }}</option>
              <option
                v-for="value in [
                  'active',
                  'maintenance',
                  'disabled',
                  'deleting',
                ] as const"
                :key="value"
                :value="value"
              >
                {{ t(`status.${value}`) }}
              </option>
            </select>
          </label>
          <span class="toolbar-note">{{ t('common.searchServer') }}</span>
        </div>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{{ t('col.project') }}</th>
                <th>{{ t('col.owner') }}</th>
                <th>{{ t('col.status') }}</th>
                <th class="numeric">{{ t('col.generation') }}</th>
                <th class="numeric">{{ t('col.sequence') }}</th>
                <th>{{ t('col.updated') }}</th>
                <th>
                  <span class="sr-only">{{ t('common.manage') }}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in filteredProjects" :key="row.id">
                <td>
                  <button class="identity-button" @click="openDrawer(row)">
                    <span class="project-icon"><AppIcon name="folder" /></span>
                    <span
                      ><strong>{{ row.name }}</strong
                      ><small class="mono">{{ shortId(row.id) }}</small></span
                    >
                  </button>
                </td>
                <td class="mono">{{ shortId(row.ownerUserId) }}</td>
                <td><StatusBadge :value="row.status" /></td>
                <td class="numeric mono">{{ row.generation }}</td>
                <td class="numeric mono">
                  {{ formatNumber(row.changeSequence) }}
                </td>
                <td class="muted">{{ formatDateTime(row.updatedAt) }}</td>
                <td>
                  <button class="text-button" @click="openDrawer(row)">
                    {{ t('common.manage') }}<AppIcon name="chevron-right" />
                  </button>
                </td>
              </tr>
              <tr v-if="!filteredProjects.length">
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
          <span>{{
            t('common.pageRows', { n: filteredProjects.length })
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
      <div class="page-note">
        <AppIcon name="history" /><span>{{ t('projects.note') }}</span>
      </div>
    </template>

    <AppDialog variant="drawer" :open="drawerOpen" @close="drawerOpen = false">
      <template v-if="drawerOpen && drawerProject">
        <header class="drawer-header">
          <div>
            <span class="eyebrow">{{ t('drawer.project') }}</span>
            <h2>{{ drawerProject.name }}</h2>
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
          <StatusBadge :value="drawerProject.status" />
          <div class="detail-grid">
            <span>{{ t('col.id') }}</span>
            <strong class="mono">{{ drawerProject.id }}</strong>
            <span>{{ t('col.owner') }}</span>
            <strong class="mono">{{ drawerProject.ownerUserId }}</strong>
            <span>{{ t('col.generation') }}</span>
            <strong>{{ drawerProject.generation }}</strong>
            <span>{{ t('col.sequence') }}</span>
            <strong>{{ formatNumber(drawerProject.changeSequence) }}</strong>
            <span>{{ t('col.created') }}</span>
            <strong>{{ formatDateTime(drawerProject.createdAt) }}</strong>
            <span>{{ t('col.updated') }}</span>
            <strong>{{ formatDateTime(drawerProject.updatedAt) }}</strong>
          </div>
          <div class="section-divider"></div>
          <h3>{{ t('projects.history') }}</h3>
          <p class="muted">{{ t('projects.historyHint') }}</p>
          <template v-if="historyLoading">
            <div class="skeleton"></div>
            <div class="skeleton"></div>
            <div class="skeleton"></div>
          </template>
          <p v-else-if="historyError" class="muted">
            {{ t('common.loadFailed') }}
          </p>
          <ol v-else-if="history.length" class="history-list">
            <li
              v-for="item in history"
              :key="`${item.objectId}-${item.revision}`"
            >
              <span class="timeline-dot"></span>
              <div>
                <strong
                  >{{ historyTitle(item)
                  }}<code> r{{ item.revision }}</code></strong
                >
                <small
                  >{{ t('col.sequence') }}
                  {{ formatNumber(item.changeSequence) }} ·
                  {{ formatDateTime(item.changedAt) }}</small
                >
                <code class="muted"
                  >{{ shortId(item.objectId) }} · {{ t('col.device') }}
                  {{ shortId(item.deviceId) }}</code
                >
              </div>
            </li>
          </ol>
          <p v-else class="muted">{{ t('states.emptyTitle') }}</p>
          <div class="notice info">
            <AppIcon name="shield" /><span>{{
              t('projects.metadataOnly')
            }}</span>
          </div>
        </div>
        <footer class="drawer-footer">
          <span
            v-if="drawerProject.status !== 'active'"
            class="muted"
            style="margin-right: auto; font-size: 11px"
          >
            {{ t('projects.restoreDisabled') }}
          </span>
          <button class="button" @click="drawerOpen = false">
            {{ t('common.close') }}
          </button>
          <button
            class="button primary"
            :disabled="drawerProject.status !== 'active'"
            @click="startRestore(drawerProject)"
          >
            <AppIcon name="history" />{{ t('restore.start') }}
          </button>
        </footer>
      </template>
    </AppDialog>

    <AppDialog :open="restoreOpen" @close="restoreOpen = false">
      <template v-if="restoreOpen && drawerProject">
        <header class="modal-header">
          <div class="modal-heading-icon"><AppIcon name="history" /></div>
          <button
            class="icon-button"
            :aria-label="t('common.close')"
            @click="restoreOpen = false"
          >
            <AppIcon name="close" />
          </button>
        </header>
        <h2>{{ t('modal.restore') }}</h2>
        <p class="muted modal-description">{{ t('modalHint.restore') }}</p>
        <form @submit.prevent="submitRestore">
          <ol class="steps">
            <li :class="{ current: restoreStep === 1 }">
              <span>1</span>{{ t('restore.target') }}
            </li>
            <li :class="{ current: restoreStep === 2 }">
              <span>2</span>{{ t('restore.review') }}
            </li>
          </ol>
          <div class="target-card">
            <AppIcon name="folder" />
            <div>
              <strong>{{ drawerProject.name }}</strong>
              <small class="mono">{{ drawerProject.id }}</small>
            </div>
          </div>
          <template v-if="restoreStep === 1">
            <label
              >{{ t('restore.type') }}
              <select v-model="targetType">
                <option value="sequence">{{ t('restore.sequence') }}</option>
                <option value="snapshot">{{ t('restore.snapshot') }}</option>
              </select></label
            >
            <label v-if="targetType === 'sequence'"
              >{{ t('restore.sequence') }}
              <input
                v-model="formSequence"
                type="number"
                min="0"
                :max="drawerProject.changeSequence"
                step="1"
                required
            /></label>
            <label v-else
              >{{ t('restore.snapshotId') }}
              <input
                v-model="formSnapshotId"
                class="mono"
                pattern="[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
                required
                autocomplete="off"
            /></label>
            <label
              >{{ t('common.reason') }}
              <textarea
                v-model="formReason"
                rows="3"
                maxlength="512"
                :placeholder="t('restore.reasonPlaceholder')"
                required
              ></textarea></label
          ></template>
          <template v-else>
            <div class="notice warning">
              <AppIcon name="alert" />{{ t('restore.impact') }}
            </div>
            <div class="review-target">
              <span>{{
                targetType === 'sequence'
                  ? t('restore.sequence')
                  : t('restore.snapshotId')
              }}</span>
              <strong class="mono">{{
                targetType === 'sequence' ? formSequence : formSnapshotId
              }}</strong>
            </div>
            <label
              >{{ t('restore.confirmId') }}
              <input
                v-model="formConfirmId"
                class="mono"
                :placeholder="drawerProject.id"
                required
                autocomplete="off"
            /></label>
          </template>
          <p v-if="restoreError" class="form-error" role="alert">
            {{ restoreError }}
          </p>
          <footer class="modal-footer">
            <button
              type="button"
              class="button"
              :disabled="restoreLoading"
              @click="backStep"
            >
              {{ restoreStep === 2 ? t('common.back') : t('common.cancel') }}
            </button>
            <button class="button primary" :disabled="restoreLoading">
              {{ restoreStep === 1 ? t('restore.next') : t('restore.submit') }}
            </button>
          </footer>
        </form>
      </template>
    </AppDialog>
  </section>
</template>
