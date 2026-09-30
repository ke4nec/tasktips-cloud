/** Audit event presentation: Chinese action names next to the raw action
 * code, and a strict metadata allowlist. Only keys emitted by this backend
 * (crates/persistence audit writes) and known to be non-payload are shown. */

/** Maps a raw audit action code to its i18n key under `auditAction.`. */
export const AUDIT_ACTION_KEYS: Record<string, string> = {
  'account.status_changed': 'account.statusChanged',
  'account.purge_requested': 'account.purgeRequested',
  'account.purge_export_requested': 'account.purgeExportRequested',
  'account.purge_export_ready': 'account.purgeExportReady',
  'account.purge_completed': 'account.purgeCompleted',
  'auth.login': 'auth.login',
  'auth.login_failed': 'auth.loginFailed',
  'auth.logout': 'auth.logout',
  'auth.refresh': 'auth.refresh',
  'auth.refresh_failed': 'auth.refreshFailed',
  'auth.reauth_failed': 'auth.reauthFailed',
  'auth.registered': 'auth.registered',
  'auth.invitation_activated': 'auth.invitationActivated',
  'device.revoked': 'device.revoked',
  'invitation.created': 'invitation.created',
  'invitation.resent': 'invitation.resent',
  'invitation.revoked': 'invitation.revoked',
  'project.purge_requested': 'project.purgeRequested',
  'restore.requested': 'restore.requested',
  'restore.cancel_requested': 'restore.cancelRequested',
  'restore.cancelled': 'restore.cancelled',
  'restore.reopened': 'restore.reopened',
  'settings.registration_changed': 'settings.registrationChanged',
  'snapshot.created': 'snapshot.created',
  'user.created_by_admin': 'user.createdByAdmin',
};

/** i18n key for an action, or undefined when the UI shows the raw code. */
export function auditActionKey(action: string): string | undefined {
  return AUDIT_ACTION_KEYS[action];
}

/** Metadata fields considered safe to render, with their i18n labels. */
const METADATA_FIELDS: Record<string, string> = {
  status: 'metadata.status',
  reason: 'metadata.reason',
  enabled: 'metadata.enabled',
  restoreId: 'metadata.restoreId',
  jobId: 'metadata.jobId',
  source: 'metadata.source',
  generation: 'metadata.generation',
  objects: 'metadata.objects',
  tombstones: 'metadata.tombstones',
  deviceId: 'metadata.deviceId',
  invitationId: 'metadata.invitationId',
  emailHash: 'metadata.emailHash',
  clientHash: 'metadata.clientHash',
};

export interface AuditMetadataRow {
  label: string;
  value: string;
}

/**
 * Extracts displayable metadata rows. Unknown keys are dropped instead of
 * rendered, so future backend fields never leak into the UI by default.
 */
export function auditMetadataRows(
  metadata: Record<string, unknown> | null | undefined,
): AuditMetadataRow[] {
  if (!metadata) return [];
  const rows: AuditMetadataRow[] = [];
  for (const [key, label] of Object.entries(METADATA_FIELDS)) {
    const value = metadata[key];
    if (value === null || value === undefined || value === '') continue;
    if (
      typeof value === 'string' ||
      typeof value === 'number' ||
      typeof value === 'boolean'
    ) {
      rows.push({ label, value: String(value) });
    }
  }
  return rows;
}
