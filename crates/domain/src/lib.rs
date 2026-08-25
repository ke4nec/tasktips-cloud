use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MIN_PASSWORD_LENGTH: usize = 12;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    User,
    SystemAdmin,
}

impl UserRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::SystemAdmin => "system_admin",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    Disabled,
    Pending,
    Deleting,
}

impl AccountStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Pending => "pending",
            Self::Deleting => "deleting",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Active,
    Maintenance,
    Disabled,
    Deleting,
}

impl ProjectStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Maintenance => "maintenance",
            Self::Disabled => "disabled",
            Self::Deleting => "deleting",
        }
    }
}

#[must_use]
pub fn normalize_email(email: &str) -> Option<String> {
    let normalized = email.trim().to_lowercase();
    let (local, domain) = normalized.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.starts_with('.') || domain.ends_with('.') {
        return None;
    }
    Some(normalized)
}

#[must_use]
pub fn valid_password(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_LENGTH
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Todo,
    Classification,
    Index,
    Image,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectIdentity {
    pub project_id: Uuid,
    pub kind: ObjectKind,
    pub object_id: String,
}
