<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';

import AppDialog from '@/components/AppDialog.vue';
import AppIcon from '@/components/AppIcon.vue';
import StatePanel from '@/components/StatePanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { ApiError, apiClient } from '@/api/client';
import type { components } from '@/api/generated/schema';
import { isValidPassword, formatDateTime, shortId } from '@/lib/format';
import { useToast } from '@/composables/useToast';

type AdminUser = components['schemas']['AdminUser'];
type ProjectList = components['schemas']['ProjectList'];
type DeviceList = components['schemas']['DeviceList'];

const { t, te } = useI18n();
const toast = useToast();

const users = ref<AdminUser[]>([]);
const loading = ref(true);
const error = ref<string>();
const requestId = ref<string>();
const pageOffset = ref(0);
const hasMore = ref(false);
const pageSize = 100;
const search = ref('');
const filter = ref<'all' | 'pending' | 'active' | 'disabled'>('all');
const AVATAR_TINTS = ['', 'mint', 'lavender', 'amber'] as const;
let loadSequence = 0;

const filteredUsers = computed(() => {
  const query = search.value.trim().toLowerCase();
  return users.value.filter((user) => {
    const groupMatch = filter.value === 'all' || user.status === filter.value;
    const queryMatch =
      !query ||
      user.email.toLowerCase().includes(query) ||
      user.id.toLowerCase().includes(query);
    return groupMatch && queryMatch;
  });
});
const pendingCount = computed(
  () => users.value.filter((user) => user.status === 'pending').length,
);

function setFilter(value: 'all' | 'pending' | 'active' | 'disabled'): void {
  filter.value = value;
}

function tint(index: number): string {
  return AVATAR_TINTS[index % AVATAR_TINTS.length];
}

async function load(): Promise<void> {
  const sequence = ++loadSequence;
  loading.value = true;
  error.value = undefined;
  requestId.value = undefined;
  try {
    const response = await apiClient.getAdminUsers({
      limit: pageSize,
      offset: pageOffset.value,
    });
    if (sequence !== loadSequence) return;
    users.value = response.items;
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

function firstPage(): void {
  pageOffset.value = 0;
  void load();
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

watch([search, filter], () => {
  if (pageOffset.value !== 0) firstPage();
});

/* Row actions and confirmation modals */
type ModalType = 'create' | 'enable' | 'disable';
const modalOpen = ref(false);
const modalType = ref<ModalType>('create');
const modalLoading = ref(false);
const modalError = ref('');
const targetUser = ref<AdminUser>();
const formEmail = ref('');
const formPassword = ref('');
const formConfirm = ref('');
const formReason = ref('');
const formConfirmId = ref('');

function openCreate(): void {
  modalType.value = 'create';
  formEmail.value = '';
  formPassword.value = '';
  formConfirm.value = '';
  modalError.value = '';
  modalOpen.value = true;
}

function openStatus(
  row: AdminUser,
  action: 'approve' | 'enable' | 'disable',
): void {
  modalType.value = action === 'disable' ? 'disable' : 'enable';
  targetUser.value = row;
  formReason.value = t(
    action === 'approve'
      ? 'users.approveReason'
      : action === 'disable'
        ? 'users.disableReason'
        : 'users.enableReason',
  );
  formConfirmId.value = '';
  modalError.value = '';
  modalOpen.value = true;
}

function rowAction(row: AdminUser): 'approve' | 'disable' | 'enable' | null {
  if (row.role === 'system_admin') return null;
  if (row.status === 'pending') return 'approve';
  if (row.status === 'active') return 'disable';
  if (row.status === 'disabled') return 'enable';
  return null;
}

function actionLabel(row: AdminUser): string {
  const action = rowAction(row);
  if (!action) return '';
  return t(`users.${action}`);
}

function openAction(row: AdminUser): void {
  const action = rowAction(row);
  if (!action) return;
  openStatus(row, action);
}

async function submitModal(): Promise<void> {
  if (modalLoading.value) return;
  modalError.value = '';
  if (modalType.value === 'create') {
    if (!isValidPassword(formPassword.value)) {
      modalError.value = t('common.invalidPassword');
      return;
    }
    if (formPassword.value !== formConfirm.value) {
      modalError.value = t('common.passwordMismatch');
      return;
    }
  } else {
    if (!formReason.value.trim()) {
      modalError.value = t('common.required');
      return;
    }
    if (
      modalType.value === 'disable' &&
      formConfirmId.value.trim() !== (targetUser.value?.id ?? '')
    ) {
      modalError.value = t('common.idMismatch');
      return;
    }
  }
  modalLoading.value = true;
  try {
    if (modalType.value === 'create') {
      await apiClient.createUser(formEmail.value.trim(), formPassword.value);
      toast.show(t('users.created'));
    } else if (targetUser.value) {
      await apiClient.setAccountStatus(
        targetUser.value.id,
        modalType.value,
        formReason.value.trim(),
      );
      toast.show(t('users.updated'));
    }
    modalOpen.value = false;
    await load();
  } catch (reason) {
    modalError.value =
      reason instanceof Error ? reason.message : t('common.loadFailed');
  } finally {
    modalLoading.value = false;
  }
}

/* Detail drawer with related resources */
const drawerOpen = ref(false);
const drawerUser = ref<AdminUser>();
const relatedProjects = ref<ProjectList>();
const relatedDevices = ref<DeviceList>();
const relatedLoading = ref(false);
let relatedSequence = 0;

async function openDrawer(row: AdminUser): Promise<void> {
  drawerUser.value = row;
  relatedProjects.value = undefined;
  relatedDevices.value = undefined;
  drawerOpen.value = true;
  const sequence = ++relatedSequence;
  relatedLoading.value = true;
  try {
    const [projects, devices] = await Promise.allSettled([
      apiClient.getAdminUserProjects(row.id, { limit: 5, offset: 0 }),
      apiClient.getAdminUserDevices(row.id, { limit: 5, offset: 0 }),
    ]);
    if (sequence !== relatedSequence) return;
    relatedProjects.value =
      projects.status === 'fulfilled' ? projects.value : undefined;
    relatedDevices.value =
      devices.status === 'fulfilled' ? devices.value : undefined;
  } finally {
    if (sequence === relatedSequence) relatedLoading.value = false;
  }
}

function roleLabel(role: string): string {
  return te(`role.${role}`) ? t(`role.${role}`) : role;
}

onMounted(load);
onBeforeUnmount(() => {
  loadSequence += 1;
  relatedSequence += 1;
});
</script>

<template>
  <section>
    <header class="page-heading">
      <div>
        <div class="eyebrow">{{ t('eyebrow.users') }}</div>
        <h1>{{ t('title.users') }}</h1>
        <p>{{ t('description.users') }}</p>
      </div>
      <div class="heading-actions">
        <button class="button primary" @click="openCreate">
          <AppIcon name="user-plus" />{{ t('users.create') }}
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
        <div class="list-tabs">
          <button
            v-for="value in ['all', 'pending', 'active', 'disabled'] as const"
            :key="value"
            :class="{ active: filter === value }"
            @click="setFilter(value)"
          >
            {{ t(`filter.${value}`) }}
            <span v-if="value === 'pending' && pendingCount">{{
              pendingCount
            }}</span>
          </button>
        </div>
        <div class="table-toolbar">
          <label class="search-field">
            <AppIcon name="search" />
            <input
              v-model="search"
              type="search"
              :placeholder="t('search.users')"
              :aria-label="t('search.users')"
            />
          </label>
          <span class="toolbar-note">{{ t('common.pageFilter') }}</span>
        </div>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>{{ t('col.account') }}</th>
                <th>{{ t('col.role') }}</th>
                <th>{{ t('col.status') }}</th>
                <th>{{ t('col.created') }}</th>
                <th>{{ t('col.lastLogin') }}</th>
                <th>
                  <span class="sr-only">{{ t('col.actions') }}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, index) in filteredUsers" :key="row.id">
                <td>
                  <button class="identity-button" @click="openDrawer(row)">
                    <span class="avatar" :class="tint(index)">{{
                      row.email.slice(0, 1).toUpperCase()
                    }}</span>
                    <span
                      ><strong>{{ row.email }}</strong
                      ><small class="mono">{{ shortId(row.id) }}</small></span
                    >
                  </button>
                </td>
                <td>{{ roleLabel(row.role) }}</td>
                <td><StatusBadge :value="row.status" /></td>
                <td class="muted">{{ formatDateTime(row.createdAt) }}</td>
                <td class="muted">
                  {{
                    row.lastLoginAt
                      ? formatDateTime(row.lastLoginAt)
                      : t('common.never')
                  }}
                </td>
                <td class="row-actions">
                  <button
                    v-if="rowAction(row)"
                    class="text-button"
                    :class="{ 'danger-text': rowAction(row) === 'disable' }"
                    @click="openAction(row)"
                  >
                    {{ actionLabel(row) }}
                  </button>
                  <button v-else class="text-button" @click="openDrawer(row)">
                    {{ t('common.details') }}
                  </button>
                </td>
              </tr>
              <tr v-if="!filteredUsers.length">
                <td colspan="6">
                  <div class="inline-empty">
                    <AppIcon name="search" />
                    <strong>{{ t('states.noMatch') }}</strong>
                    <button
                      class="text-button"
                      @click="
                        search = '';
                        setFilter('all');
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
          <span>{{ t('common.pageRows', { n: filteredUsers.length }) }}</span>
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

    <AppDialog :open="modalOpen" @close="modalOpen = false">
      <template v-if="modalOpen">
        <header class="modal-header">
          <div
            class="modal-heading-icon"
            :class="{ destructive: modalType === 'disable' }"
          >
            <AppIcon :name="modalType === 'create' ? 'user-plus' : 'shield'" />
          </div>
          <button
            class="icon-button"
            :aria-label="t('common.close')"
            @click="modalOpen = false"
          >
            <AppIcon name="close" />
          </button>
        </header>
        <h2>{{ t(`modal.${modalType}`) }}</h2>
        <p class="muted modal-description">{{ t(`modalHint.${modalType}`) }}</p>
        <form @submit.prevent="submitModal">
          <template v-if="modalType === 'create'">
            <label
              >{{ t('auth.email') }}
              <input
                v-model="formEmail"
                type="email"
                autocomplete="off"
                required
            /></label>
            <label
              >{{ t('auth.password') }}
              <input
                v-model="formPassword"
                type="password"
                minlength="12"
                autocomplete="new-password"
                required
            /></label>
            <small class="field-hint">{{ t('auth.passwordHint') }}</small>
            <label
              >{{ t('auth.confirmPassword') }}
              <input
                v-model="formConfirm"
                type="password"
                autocomplete="new-password"
                required
            /></label>
          </template>
          <template v-else>
            <div class="target-card">
              <span class="avatar">{{
                (targetUser?.email ?? '?').slice(0, 1).toUpperCase()
              }}</span>
              <div>
                <strong>{{ targetUser?.email }}</strong>
                <small class="mono">{{ targetUser?.id }}</small>
              </div>
            </div>
            <label
              >{{ t('common.reason') }}
              <textarea
                v-model="formReason"
                rows="3"
                maxlength="512"
                required
              ></textarea></label
            ><label v-if="modalType === 'disable'"
              >{{ t('users.confirmId') }}
              <input
                v-model="formConfirmId"
                class="mono"
                :placeholder="targetUser?.id"
                required
                autocomplete="off"
            /></label>
          </template>
          <p v-if="modalError" class="form-error" role="alert">
            {{ modalError }}
          </p>
          <footer class="modal-footer">
            <button
              type="button"
              class="button"
              :disabled="modalLoading"
              @click="modalOpen = false"
            >
              {{ t('common.cancel') }}
            </button>
            <button
              class="button"
              :class="modalType === 'disable' ? 'danger-button' : 'primary'"
              :disabled="modalLoading"
            >
              {{ t('common.confirm') }}
            </button>
          </footer>
        </form>
      </template>
    </AppDialog>

    <AppDialog variant="drawer" :open="drawerOpen" @close="drawerOpen = false">
      <template v-if="drawerOpen && drawerUser">
        <header class="drawer-header">
          <div>
            <span class="eyebrow">{{ t('drawer.user') }}</span>
            <h2>{{ drawerUser.email }}</h2>
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
          <StatusBadge :value="drawerUser.status" />
          <div class="detail-grid">
            <span>{{ t('col.id') }}</span>
            <strong class="mono">{{ drawerUser.id }}</strong>
            <span>{{ t('col.role') }}</span>
            <strong>{{ roleLabel(drawerUser.role) }}</strong>
            <span>{{ t('col.created') }}</span>
            <strong>{{ formatDateTime(drawerUser.createdAt) }}</strong>
            <span>{{ t('col.lastLogin') }}</span>
            <strong>
              {{
                drawerUser.lastLoginAt
                  ? formatDateTime(drawerUser.lastLoginAt)
                  : t('common.never')
              }}
            </strong>
          </div>
          <div class="section-divider"></div>
          <h3>{{ t('users.related') }}</h3>
          <p class="muted">{{ t('users.relatedHint') }}</p>
          <template v-if="relatedLoading">
            <div class="skeleton"></div>
            <div class="skeleton"></div>
          </template>
          <template v-else>
            <div class="related-link">
              <AppIcon name="folder" />
              <span>
                {{ t('users.relatedProjects') }}
                <span v-if="relatedProjects" class="muted">
                  ·
                  {{
                    relatedProjects.items.length +
                    (relatedProjects.hasMore ? '+' : '')
                  }}
                </span>
              </span>
            </div>
            <div
              v-for="project in relatedProjects?.items ?? []"
              :key="project.id"
              class="related-item"
            >
              <strong>{{ project.name }}</strong>
              <span class="mono muted">{{ shortId(project.id) }}</span>
            </div>
            <div class="related-link">
              <AppIcon name="monitor" />
              <span>
                {{ t('users.relatedDevices') }}
                <span v-if="relatedDevices" class="muted">
                  ·
                  {{
                    relatedDevices.items.length +
                    (relatedDevices.hasMore ? '+' : '')
                  }}
                </span>
              </span>
            </div>
            <div
              v-for="device in relatedDevices?.items ?? []"
              :key="device.id"
              class="related-item"
            >
              <strong>{{ device.displayName }}</strong>
              <span class="mono muted">{{ shortId(device.id) }}</span>
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
