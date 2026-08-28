pub use tasktips_domain::{
    AccountStatus, MIN_PASSWORD_LENGTH, ObjectIdentity, ObjectKind, ProjectStatus,
    SyncObjectMetadata, SyncTombstoneMetadata, UserRole, normalize_email, valid_password,
};

use uuid::Uuid;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PolicyError {
    InvalidRestoreTarget,
    InvalidAccountPurgePhase,
}

/// Validates the mutually exclusive restore targets shared by HTTP and persistence adapters.
///
/// # Errors
///
/// Returns `InvalidRestoreTarget` when neither or both target forms are supplied,
/// or when the sequence is negative.
#[allow(clippy::missing_errors_doc)]
pub fn validate_restore_target(
    snapshot_id: Option<Uuid>,
    target_change_sequence: Option<i64>,
) -> Result<(), PolicyError> {
    if snapshot_id.is_none() == target_change_sequence.is_none()
        || target_change_sequence.is_some_and(|sequence| sequence < 0)
    {
        return Err(PolicyError::InvalidRestoreTarget);
    }
    Ok(())
}

/// Validates the two-phase account purge request shape before it reaches a storage adapter.
///
/// # Errors
///
/// Returns `InvalidAccountPurgePhase` when the confirmation flag and export ID
/// do not describe the same phase.
#[allow(clippy::missing_errors_doc)]
pub fn validate_account_purge_phase(
    export_id: Option<Uuid>,
    confirmed: bool,
) -> Result<(), PolicyError> {
    if confirmed != export_id.is_some() {
        return Err(PolicyError::InvalidAccountPurgePhase);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PolicyError, validate_account_purge_phase, validate_restore_target};
    use uuid::Uuid;

    #[test]
    fn restore_target_is_exactly_one_mode() {
        assert!(validate_restore_target(Some(Uuid::nil()), None).is_ok());
        assert!(validate_restore_target(None, Some(0)).is_ok());
        assert_eq!(
            validate_restore_target(Some(Uuid::nil()), Some(0)),
            Err(PolicyError::InvalidRestoreTarget)
        );
        assert_eq!(
            validate_restore_target(None, Some(-1)),
            Err(PolicyError::InvalidRestoreTarget)
        );
    }

    #[test]
    fn account_purge_confirmation_matches_export_id() {
        assert!(validate_account_purge_phase(None, false).is_ok());
        assert!(validate_account_purge_phase(Some(Uuid::nil()), true).is_ok());
        assert_eq!(
            validate_account_purge_phase(None, true),
            Err(PolicyError::InvalidAccountPurgePhase)
        );
    }
}
