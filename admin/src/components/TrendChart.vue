<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';

import { formatNumber, formatTrendDayShort } from '@/lib/format';

export interface TrendPoint {
  day: string;
  succeeded: number;
}

const props = defineProps<{ points: TrendPoint[]; days: number }>();
const { t } = useI18n();

const WIDTH = 760;
const HEIGHT = 186;
const LEFT = 44;
const RIGHT = 746;
const TOP = 18;
const BOTTOM = 150;

/** A rounded grid maximum so the axis stays readable for any data scale. */
const gridMax = computed(() => {
  const peak = Math.max(1, ...props.points.map((point) => point.succeeded));
  const magnitude = 10 ** Math.floor(Math.log10(peak));
  return Math.ceil(peak / magnitude) * magnitude;
});

const gridLines = computed(() =>
  [0, 1, 2, 3].map((index) => ({
    y: TOP + index * 44,
    label: formatNumber(Math.round((gridMax.value * (3 - index)) / 3)),
  })),
);

const chartPoints = computed(() =>
  props.points.map((point, index, array) => ({
    x:
      array.length === 1
        ? (LEFT + RIGHT) / 2
        : LEFT + (index * (RIGHT - LEFT)) / (array.length - 1),
    y: BOTTOM - (point.succeeded / gridMax.value) * (BOTTOM - TOP),
  })),
);

const chartLine = computed(() =>
  chartPoints.value
    .map((point, index) => `${index ? 'L' : 'M'}${point.x},${point.y}`)
    .join(' '),
);

const chartArea = computed(() => {
  const points = chartPoints.value;
  if (!points.length) return '';
  const last = points[points.length - 1]!;
  const first = points[0]!;
  return `${chartLine.value} L${last.x},${BOTTOM} L${first.x},${BOTTOM} Z`;
});

const xLabels = computed(() => {
  const count = props.points.length;
  const step = count > 4 ? Math.ceil(count / 5) : 0;
  return props.points
    .map((point, index) => ({
      index,
      x: chartPoints.value[index]?.x ?? 0,
      label: formatTrendDayShort(point.day).slice(5),
    }))
    .filter(
      (label) =>
        label.index === 0 ||
        label.index === count - 1 ||
        (step > 0 && label.index % step === 0),
    );
});
</script>

<template>
  <svg
    class="trend-chart"
    :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
    role="img"
    :aria-label="t('trend.chartLabel', { n: days })"
  >
    <defs>
      <linearGradient id="chart-fill" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="var(--accent)" stop-opacity=".19" />
        <stop offset="100%" stop-color="var(--accent)" stop-opacity=".01" />
      </linearGradient>
    </defs>
    <g v-for="line in gridLines" :key="line.y">
      <line
        :x1="LEFT"
        :x2="RIGHT"
        :y1="line.y"
        :y2="line.y"
        class="chart-grid"
      />
      <text :x="0" :y="line.y + 4" class="chart-label">{{ line.label }}</text>
    </g>
    <path :d="chartArea" fill="url(#chart-fill)" />
    <path
      :d="chartLine"
      fill="none"
      stroke="var(--accent)"
      stroke-width="2.4"
      stroke-linejoin="round"
      stroke-linecap="round"
    />
    <circle
      v-for="(point, index) in chartPoints"
      :key="index"
      :cx="point.x"
      :cy="point.y"
      r="3"
      fill="var(--surface)"
      stroke="var(--accent)"
      stroke-width="1.8"
    >
      <title>
        {{ points[index]?.day }} ·
        {{ formatNumber(points[index]?.succeeded ?? 0) }}
      </title>
    </circle>
    <g v-for="label in xLabels" :key="label.index">
      <text
        :x="label.x"
        :y="HEIGHT - 8"
        text-anchor="middle"
        class="chart-label"
      >
        {{ label.label }}
      </text>
    </g>
  </svg>
</template>
