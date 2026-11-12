use std::vec;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct StorageConfig {
    /// If previous channels switched to should be tracked.
    pub channels: bool,
    /// If previous username mentions should be tracked.
    pub mentions: bool,
}

impl From<StorageConfig> for Vec<(String, String)> {
    fn from(config: StorageConfig) -> Self {
        vec![
            ("Channels enabled".to_string(), config.channels.to_string()),
            ("Mentions enabled".to_string(), config.mentions.to_string()),
        ]
    }
}
