//! Operations every broker exposes: `broker.ping` and `broker.events.poll`.

use crate::envelope::texts;
use crate::events::{EventHub, EventsPage};
use crate::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use crate::protocol::{CAPABILITIES, PROTOCOL_VERSION};
use bitbybit_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PingResponse {
    pub broker_version: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
    pub operations: Vec<String>,
    pub developer_mode: bool,
}

pub struct Ping {
    descriptor: Descriptor,
    version: String,
    developer_mode: bool,
    /// Filled with the registry's names once it is frozen.
    pub operations: Arc<OnceLock<Vec<String>>>,
}

impl Ping {
    pub fn new(version: &str, developer_mode: bool) -> Self {
        Self {
            descriptor: Descriptor {
                name: "broker.ping",
                tier: RiskTier::ReadOnly,
                category: Category::NONE,
                requires_confirmation: false,
                timeout: None,
            },
            version: version.to_owned(),
            developer_mode,
            operations: Arc::new(OnceLock::new()),
        }
    }
}

impl BrokerOperation for Ping {
    type Request = Empty;
    type Response = PingResponse;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &Empty) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &Empty, _: &CallContext) -> Result<PingResponse> {
        Ok(PingResponse {
            broker_version: self.version.clone(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: CAPABILITIES.iter().map(|c| (*c).to_owned()).collect(),
            operations: self.operations.get().cloned().unwrap_or_default(),
            developer_mode: self.developer_mode,
        })
    }
}

pub const MAX_POLL_WAIT_SECONDS: u32 = 25;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PollRequest {
    #[serde(default)]
    pub after_sequence: u64,
    #[serde(default = "hundred")]
    pub max_events: u32,
    #[serde(default = "twenty")]
    pub max_wait_seconds: u32,
}

fn hundred() -> u32 {
    100
}
fn twenty() -> u32 {
    20
}

pub struct PollEvents {
    descriptor: Descriptor,
    hub: Arc<EventHub>,
}

impl PollEvents {
    pub fn new(hub: Arc<EventHub>) -> Self {
        let timeout = Some(Duration::from_secs(u64::from(MAX_POLL_WAIT_SECONDS) + 5));
        Self {
            descriptor: Descriptor {
                name: "broker.events.poll",
                tier: RiskTier::ReadOnly,
                category: Category::NONE,
                requires_confirmation: false,
                timeout,
            },
            hub,
        }
    }
}

impl BrokerOperation for PollEvents {
    type Request = PollRequest;
    type Response = EventsPage;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, r: &PollRequest) -> Result<()> {
        if !(1..=500).contains(&r.max_events) || r.max_wait_seconds > MAX_POLL_WAIT_SECONDS {
            return Err(Error::invalid("Invalid poll parameters.").with_user_text(texts::malformed_request()));
        }
        Ok(())
    }
    fn execute(&self, r: &PollRequest, _: &CallContext) -> Result<EventsPage> {
        Ok(self.hub.poll(r.after_sequence, r.max_events as usize, Duration::from_secs(u64::from(r.max_wait_seconds))))
    }
}
