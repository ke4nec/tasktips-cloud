import { describe, expect, it } from 'vitest';

import { auditActionKey, auditMetadataRows } from './audit';

describe('auditActionKey', () => {
  it('maps known backend action codes to i18n keys', () => {
    expect(auditActionKey('account.status_changed')).toBe(
      'account.statusChanged',
    );
    expect(auditActionKey('settings.registration_changed')).toBe(
      'settings.registrationChanged',
    );
    expect(auditActionKey('restore.requested')).toBe('restore.requested');
    expect(auditActionKey('user.created_by_admin')).toBe('user.createdByAdmin');
  });

  it('leaves unknown codes unmapped so the raw code is displayed', () => {
    expect(auditActionKey('future.action')).toBeUndefined();
  });
});

describe('auditMetadataRows', () => {
  it('extracts only allowlisted scalar fields', () => {
    expect(
      auditMetadataRows({
        status: 'disabled',
        reason: 'abuse',
        nested: { secret: 'x' },
        array: ['a'],
        unknown: 'drop-me',
      }),
    ).toEqual([
      { label: 'metadata.status', value: 'disabled' },
      { label: 'metadata.reason', value: 'abuse' },
    ]);
  });

  it('skips empty values and handles missing metadata', () => {
    expect(auditMetadataRows({ status: '', reason: null })).toEqual([]);
    expect(auditMetadataRows(undefined)).toEqual([]);
    expect(auditMetadataRows(null)).toEqual([]);
  });

  it('keeps boolean and numeric allowlisted values', () => {
    expect(auditMetadataRows({ enabled: false, generation: 3 })).toEqual([
      { label: 'metadata.enabled', value: 'false' },
      { label: 'metadata.generation', value: '3' },
    ]);
  });
});
