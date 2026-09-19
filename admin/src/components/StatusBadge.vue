<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

const props = defineProps<{ value: string }>();
const { t, te } = useI18n();

const TONES: Record<string, string> = {
  active: 'success',
  succeeded: 'success',
  pending: 'warning',
  maintenance: 'warning',
  conflict: 'warning',
  running: 'info',
  queued: 'info',
  failed: 'danger',
  deleting: 'danger',
};

const tone = computed(() => TONES[props.value] ?? '');
const label = computed(() =>
  te(`status.${props.value}`) ? t(`status.${props.value}`) : props.value,
);
</script>

<template>
  <span class="badge" :class="tone">{{ label }}</span>
</template>
