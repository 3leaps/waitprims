//! Local scripted-event documents. Not an `agent-wait/v0` message kind.

use serde::Deserialize;
use waitprims_core::{Error, WaitEvent};

/// Scripted observations for a diagnostic first-match.
#[derive(Debug, Clone, Deserialize)]
pub struct Script {
    /// Per-arm bound. Overflow is typed (`partial` or `failed`).
    #[serde(default = "default_buffer_limit")]
    pub buffer_limit: usize,
    /// Events in any order. `observed_at` is the ready time.
    #[serde(default)]
    pub events: Vec<WaitEvent>,
}

fn default_buffer_limit() -> usize {
    8
}

impl Script {
    /// Parse a local script. Accepts `{"events":[...]}` or a bare event array.
    pub fn from_json(raw: &str) -> Result<Self, Error> {
        let value: serde_json::Value =
            serde_json::from_str(raw).map_err(|_| Error::MalformedJson)?;
        if value.is_array() {
            let events: Vec<WaitEvent> =
                serde_json::from_value(value).map_err(|_| Error::MalformedJson)?;
            return Ok(Self {
                buffer_limit: default_buffer_limit(),
                events,
            });
        }
        serde_json::from_value(value).map_err(|_| Error::MalformedJson)
    }
}

impl Default for Script {
    fn default() -> Self {
        Self {
            buffer_limit: default_buffer_limit(),
            events: Vec::new(),
        }
    }
}
