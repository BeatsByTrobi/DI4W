//! What the island reports back to the settings app (`status.json`): per add-on state keyed by the add-on's
//! settings name ("Weather", "Calendar"…), and Windows notification access.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One add-on's state as the island sees it, e.g. "Set a city to show the weather".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AddonState {
    pub running: bool,
    pub message: Option<String>,
    /// True when the add-on can't do its job until the user does something.
    pub problem: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct IslandStatus {
    pub addons: BTreeMap<String, AddonState>,
    /// Windows notification access: "Allowed", "Denied", "Unspecified" or an error.
    pub notification_access: Option<String>,
}

impl IslandStatus {
    pub fn try_load() -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(crate::paths::status_file()).ok()?).ok()
    }

    /// Status is a nicety; a failed save never breaks the island.
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = crate::paths::write_atomic(&crate::paths::status_file(), json.as_bytes());
        }
    }
}
