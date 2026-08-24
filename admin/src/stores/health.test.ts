import { describe, expect, it } from 'vitest';

import { healthLabelKey } from './health';

describe('healthLabelKey', () => {
  it('maps every health state to an i18n key', () => {
    expect(healthLabelKey('checking')).toBe('app.checking');
    expect(healthLabelKey('live')).toBe('app.apiLive');
    expect(healthLabelKey('unavailable')).toBe('app.apiUnavailable');
  });
});
