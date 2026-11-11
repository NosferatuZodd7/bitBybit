//! Allowlisted broker operations and the closed registry. Operations are registered explicitly at startup (no
//! discovery, no plug-ins); the registry is frozen into an immutable value before the broker accepts connections.
//!
//! Rules for implementations: typed request structs only (no free-form `Value` members), re-validate every
//! parameter, never accept command lines, scripts, executable paths or arbitrary registry paths, build a
//! deterministic plan for mutating operations, strip secrets in `redact_for_audit`.

use crate::identity::ClientIdentity;
use crate::json;
use crate::plan::OperationPlan;
use bitbybit_core::{Error, ErrorCode, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RiskTier {
    /// Reads only. Any genuine bitBybit client of an interactive user.
    ReadOnly,
    /// Changes within bitBybit-owned scope (sharing, default audio device…).
    Standard,
    /// Drivers, devices, policies, updates: the client token must be elevated (UAC = OS-level consent).
    Elevated,
}

pub const MAX_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// At most one mutating operation per category runs at a time. `None` only for read-only operations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Category(Option<&'static str>);

impl Category {
    pub const NONE: Self = Self(None);
    pub const NETWORK: Self = Self(Some("network"));
    pub const AUDIO: Self = Self(Some("audio"));
    pub const DISPLAY: Self = Self(Some("display"));
    pub const DEVICES: Self = Self(Some("devices"));
    pub const DRIVERS: Self = Self(Some("drivers"));
    pub const FIREWALL: Self = Self(Some("firewall"));
    pub const UPDATES: Self = Self(Some("updates"));
    pub const BROKER: Self = Self(Some("broker"));

    pub fn name(&self) -> &'static str {
        self.0.unwrap_or("none")
    }
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static str,
    pub tier: RiskTier,
    pub category: Category,
    /// Always two-phase. Plans with Medium+ risk, irreversible effects or a reboot are confirmed regardless.
    pub requires_confirmation: bool,
    pub timeout: Option<Duration>,
}

impl Descriptor {
    pub fn is_mutating(&self) -> bool {
        self.tier != RiskTier::ReadOnly
    }

    fn check(&self) -> std::result::Result<(), String> {
        if !crate::envelope::is_valid_operation_name(self.name) {
            return Err(format!("operation name '{}' is not valid", self.name));
        }
        if self.is_mutating() && self.category.is_none() {
            return Err(format!("mutating operation '{}' must declare a concurrency category", self.name));
        }
        if self.timeout.is_some_and(|t| t.is_zero() || t > MAX_TIMEOUT) {
            return Err(format!("timeout of '{}' must be positive and at most 30 minutes", self.name));
        }
        Ok(())
    }
}

/// Per-call information. Cancellation is cooperative: operations check `should_stop`.
#[derive(Debug, Clone)]
pub struct CallContext {
    pub request_id: Uuid,
    pub operation: String,
    pub client: ClientIdentity,
    /// True when the two-phase confirmation completed for this call.
    pub is_confirmed: bool,
    pub received_at: OffsetDateTime,
    pub deadline: Instant,
    /// Set when the client connection closed. Mutating operations should NOT abort on it (no half-applied state).
    pub connection_closed: Arc<AtomicBool>,
}

impl CallContext {
    pub fn timed_out(&self) -> bool {
        Instant::now() >= self.deadline
    }
    pub fn client_gone(&self) -> bool {
        self.connection_closed.load(Ordering::Relaxed)
    }
}

/// An allowlisted operation.
pub trait BrokerOperation: Send + Sync {
    type Request: DeserializeOwned + Serialize + Clone;
    type Response: Serialize;
    fn descriptor(&self) -> &Descriptor;
    /// Pure parameter validation; failures carry a `LocalizedText`.
    fn validate(&self, request: &Self::Request) -> Result<()>;
    /// What `execute` would do, from the live state. Must not change anything. Required when
    /// `requires_confirmation` is set.
    fn plan(&self, _request: &Self::Request, _context: &CallContext) -> Result<Option<OperationPlan>> {
        Ok(None)
    }
    fn execute(&self, request: &Self::Request, context: &CallContext) -> Result<Self::Response>;
    /// Copy safe for the audit log.
    fn redact_for_audit(&self, request: &Self::Request) -> Self::Request {
        request.clone()
    }
}

/// Type-erased view used by the dispatcher. Requests travel as canonical bytes between phases.
pub(crate) trait Erased: Send + Sync {
    fn descriptor(&self) -> &Descriptor;
    fn bind(&self, parameters: &Value) -> Result<Vec<u8>>;
    fn validate(&self, canonical: &[u8]) -> Result<()>;
    fn plan(&self, canonical: &[u8], context: &CallContext) -> Result<Option<OperationPlan>>;
    fn execute(&self, canonical: &[u8], context: &CallContext) -> Result<Value>;
    fn audit(&self, canonical: &[u8], max_chars: usize) -> String;
}

struct Binding<O>(O);

impl<O: BrokerOperation> Binding<O> {
    fn typed(canonical: &[u8]) -> Result<O::Request> {
        Ok(serde_json::from_slice(canonical)?)
    }
}

impl<O: BrokerOperation> Erased for Binding<O> {
    fn descriptor(&self) -> &Descriptor {
        self.0.descriptor()
    }
    fn bind(&self, parameters: &Value) -> Result<Vec<u8>> {
        if !parameters.is_object() {
            return Err(Error::invalid("parameters must be a JSON object."));
        }
        json::bind::<O::Request>(parameters).map(|(_, canonical)| canonical)
    }
    fn validate(&self, canonical: &[u8]) -> Result<()> {
        self.0.validate(&Self::typed(canonical)?)
    }
    fn plan(&self, canonical: &[u8], context: &CallContext) -> Result<Option<OperationPlan>> {
        self.0.plan(&Self::typed(canonical)?, context)
    }
    fn execute(&self, canonical: &[u8], context: &CallContext) -> Result<Value> {
        let value = serde_json::to_value(self.0.execute(&Self::typed(canonical)?, context)?)?;
        if value.is_object() {
            Ok(value)
        } else {
            Err(Error::new(
                ErrorCode::PreconditionFailed,
                format!("Operation '{}' must return a JSON object.", self.descriptor().name),
            ))
        }
    }
    fn audit(&self, canonical: &[u8], max_chars: usize) -> String {
        match Self::typed(canonical).and_then(|r| Ok(serde_json::to_value(self.0.redact_for_audit(&r))?)) {
            Ok(v) => crate::audit::describe(&v, max_chars),
            Err(_) => "<unprintable>".into(),
        }
    }
}

/// Collects operations at startup. Duplicates (also case-insensitively) and invalid descriptors are refused.
#[derive(Default)]
pub struct RegistryBuilder {
    ops: BTreeMap<&'static str, Box<dyn Erased>>,
    lower: HashSet<String>,
}

impl RegistryBuilder {
    pub fn register<O: BrokerOperation + 'static>(mut self, operation: O) -> std::result::Result<Self, String> {
        let d = operation.descriptor().clone();
        d.check()?;
        if !self.lower.insert(d.name.to_lowercase()) {
            return Err(format!("an operation named '{}' (or differing only in case) is already registered", d.name));
        }
        self.ops.insert(d.name, Box::new(Binding(operation)));
        Ok(self)
    }

    /// The immutable allowlist. Nothing can be added afterwards.
    pub fn freeze(self) -> Registry {
        Registry { ops: self.ops }
    }
}

pub struct Registry {
    ops: BTreeMap<&'static str, Box<dyn Erased>>,
}

impl Registry {
    pub fn names(&self) -> Vec<String> {
        self.ops.keys().map(|k| (*k).to_owned()).collect()
    }
    pub fn descriptor(&self, name: &str) -> Option<&Descriptor> {
        self.ops.get(name).map(|o| o.descriptor())
    }
    pub(crate) fn get(&self, name: &str) -> Option<&dyn Erased> {
        self.ops.get(name).map(AsRef::as_ref)
    }
}
