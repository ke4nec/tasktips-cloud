import { describe, expect, it } from 'vitest';

import {
  formatBytes,
  formatDateTime,
  formatLatency,
  formatNumber,
  formatRate,
  isValidPassword,
  shortId,
  splitBytes,
} from './format';

describe('shortId', () => {
  it('trims UUIDs to the first segment', () => {
    expect(shortId('f8a2c001-6d41-4c9a-8e5b-010000000001')).toBe('f8a2c001');
  });

  it('keeps non-UUID identifiers intact and renders null as a dash', () => {
    expect(shortId('request-42')).toBe('request-42');
    expect(shortId(null)).toBe('—');
    expect(shortId(undefined)).toBe('—');
  });
});

describe('formatNumber', () => {
  it('groups digits with separators', () => {
    expect(formatNumber(24816)).toBe('24,816');
  });
});

describe('formatDateTime', () => {
  it('renders null-ish values as a dash', () => {
    expect(formatDateTime(null)).toBe('—');
    expect(formatDateTime(undefined)).toBe('—');
  });

  it('formats an ISO instant', () => {
    const result = formatDateTime('2026-09-19T10:24:00Z');
    expect(result).toMatch(/^\d{4}\/\d{2}\/\d{2} \d{2}:\d{2}$/);
  });

  it('normalizes the space-separated timestamp with second offsets', () => {
    expect(formatDateTime('2026-08-27 07:40:37.838146 +00:00:00')).toBe(
      '2026/08/27 15:40',
    );
  });

  it('passes through values that are not parseable dates', () => {
    expect(formatDateTime('not-a-date')).toBe('not-a-date');
  });
});

describe('formatLatency / formatRate', () => {
  it('keeps missing latency visible as a dash', () => {
    expect(formatLatency(null)).toBe('—');
    expect(formatLatency(42)).toBe('42 ms');
  });

  it('returns null for empty windows instead of a fake 100%', () => {
    expect(formatRate(0, 0)).toBeNull();
    expect(formatRate(1000, 980)).toBe('98.00');
  });
});

describe('formatBytes / splitBytes', () => {
  it('scales to GiB for large payloads', () => {
    expect(formatBytes(9 * 1024 ** 3)).toBe('9.0 GiB');
    expect(formatBytes(512)).toBe('512 B');
  });

  it('splits the metric value from its unit', () => {
    expect(splitBytes(8.42 * 1024 ** 3)).toEqual({
      value: '8.4',
      unit: 'GiB',
    });
  });
});

describe('isValidPassword', () => {
  it('requires 12+ characters with letters and digits', () => {
    expect(isValidPassword('password-twelve1')).toBe(true);
    expect(isValidPassword('short1a')).toBe(false);
    expect(isValidPassword('onlyletterspassword')).toBe(false);
    expect(isValidPassword('123456789012')).toBe(false);
  });
});
