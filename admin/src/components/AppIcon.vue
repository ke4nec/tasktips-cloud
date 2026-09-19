<script setup lang="ts">
import { computed } from 'vue';

import { iconPaths } from './icons';

const props = defineProps<{ name: string }>();
const paths = computed(() => iconPaths[props.name] ?? iconPaths.info ?? '');
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html — static, code-owned icon paths -->
  <svg class="icon" viewBox="0 0 24 24" aria-hidden="true" v-html="paths"></svg>
</template>
