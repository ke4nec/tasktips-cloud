/** Display helpers shared by admin views. Formatting never depends on
 * payload data: inputs are operational metadata values from the API. */

const numberFormatter = new Intl.NumberFormat('zh-CN');

export function formatNumber(value: number): string {
  return numberFormatter.format(value);
}

/** Full UUIDs stay in drawers and confirm dialogs; lists show the short form. */
export function shortId(value: string | null | undefined): string {
  if (!value) return '—';
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
    value,
  )
    ? value.slice(0, 8)
    : value;
}

function pad(value: number): string {
  return String(value).padStart(2, '0');
}

/**
 * The API serializes timestamps as "2026-08-27 07:40:37.838146 +00:00:00"
 * (space separator, offset with seconds), which `new Date()` rejects.
 * Normalize to RFC 3339 before parsing; pass anything else through.
 */
function parseInstant(value: string): Date | null {
  const normalized = value
    .trim()
    .replace(' ', 'T')
    .replace(/\s*([+-]\d{2}:\d{2})(?::\d{2})?$/, '$1');
  const date = new Date(normalized);
  return Number.isNaN(date.getTime()) ? null : date;
}

/** Local "YYYY/MM/DD HH:mm" for instants shown in lists and drawers. */
export function formatDateTime(value: string | null | undefined): string {
  if (!value) return '—';
  const date = parseInstant(value);
  if (!date) return value;
  return (
    `${date.getFullYear()}/${pad(date.getMonth() + 1)}/${pad(date.getDate())}` +
    ` ${pad(date.getHours())}:${pad(date.getMinutes())}`
  );
}

/** Trend buckets are UTC calendar days; show the UTC date, not local. */
export function formatTrendDay(value: string): string {
  return value.slice(0, 10);
}

export function formatTrendDayShort(value: string): string {
  return formatTrendDay(value).replaceAll('-', '/');
}

export function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  if (value < 1024 * 1024 * 1024)
    return `${(value / 1024 / 1024).toFixed(1)} MiB`;
  return `${(value / 1024 / 1024 / 1024).toFixed(1)} GiB`;
}

/** Splits "8.4 GiB" into number and unit for the metric card layout. */
export function splitBytes(value: number): { value: string; unit: string } {
  const text = formatBytes(value);
  const match = text.match(/^([0-9.]+)\s*(.+)$/);
  if (!match) return { value: text, unit: '' };
  return { value: match[1]!, unit: match[2]! };
}

export function formatLatency(value: number | null | undefined): string {
  return value === null || value === undefined ? '—' : `${value} ms`;
}

/** Percentages come from server-aggregated days; empty windows stay "—". */
export function formatRate(attempts: number, succeeded: number): string | null {
  if (attempts <= 0) return null;
  return ((100 * succeeded) / attempts).toFixed(2);
}

/** Password rule shared by registration and admin user creation. */
export function isValidPassword(value: string): boolean {
  return value.length >= 12 && /[a-z]/i.test(value) && /[0-9]/.test(value);
}
