use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable identity of an authored dock item.
#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DockItemId(String);

impl DockItemId {
    /// Creates an item identity from its authored string.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl From<&str> for DockItemId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DockItemId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for DockItemId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DockItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable identity of an authored dock group.
#[derive(Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DockGroupId(String);

impl DockGroupId {
    /// Creates a group identity from its authored string.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl From<&str> for DockGroupId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DockGroupId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for DockGroupId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DockGroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
