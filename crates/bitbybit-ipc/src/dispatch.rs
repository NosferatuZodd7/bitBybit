//! The broker's request pipeline: envelope check → allowlist lookup → authorization by tier → strict binding →
//! validation → server-side two-phase confirmation → per-category gate → execution → audit. A panic inside an
//! operation is logged server-side and returned as a generic error (no details leak to the client).

use crate::auth::AuthorizationPolicy;
use crate::confirm::TokenService;
use crate::envelope::{
    BrokerRequest, BrokerResponse, ResponseStatus, sanitize_operation_name, texts, validate_request,
};
use crate::gate::ConcurrencyGate;
use crate::identity::ClientIdentity;
use crate::operation::{CallContext, Registry};
use bitbybit_core::{Error, ErrorCode};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct DispatcherOptions {
    pub default_timeout: Duration,
    /// How long a mutating request waits for its category before failing with Busy.
    pub busy_wait: Duration,
    pub audit_parameter_chars: usize,
}

impl Default for DispatcherOptions {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(120),
            busy_wait: Duration::from_secs(15),
            audit_parameter_chars: 512,
        }
    }
}

pub struct Dispatcher {
    registry: Registry,
    policy: AuthorizationPolicy,
    tokens: TokenService,
    gate: ConcurrencyGate,
    options: DispatcherOptions,
}

const AUDIT: &str = "bitbybit::audit";

impl Dispatcher {
    pub fn new(
        registry: Registry,
        policy: AuthorizationPolicy,
        tokens: TokenService,
        options: DispatcherOptions,
    ) -> Self {
        Self { registry, policy, tokens, gate: ConcurrencyGate::default(), options }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn dispatch(
        &self,
        request: &BrokerRequest,
        client: &ClientIdentity,
        connection_closed: Arc<AtomicBool>,
    ) -> BrokerResponse {
        let started = Instant::now();
        let op = sanitize_operation_name(&request.operation);
        let mut parameters = String::from("-");
        let response = match catch_unwind(AssertUnwindSafe(|| {
            self.dispatch_core(request, client, connection_closed, &mut parameters)
        })) {
            Ok(r) => r,
            Err(_) => {
                log::error!(target: AUDIT, "Operation {op} panicked (client {client})");
                BrokerResponse::failure(
                    request.request_id,
                    Error::new(ErrorCode::PlatformError, "Internal broker error.").with_user_text(texts::internal()),
                )
            }
        };
        let outcome = match (&response.status, &response.error) {
            (ResponseStatus::Error, Some(e)) => format!("Error({:?})", e.code),
            (s, _) => format!("{s:?}"),
        };
        log::info!(target: AUDIT, "AUDIT op={op} outcome={outcome} ms={} client=[{client}] params={parameters}", started.elapsed().as_millis());
        response
    }

    fn dispatch_core(
        &self,
        request: &BrokerRequest,
        client: &ClientIdentity,
        connection_closed: Arc<AtomicBool>,
        audit_parameters: &mut String,
    ) -> BrokerResponse {
        let id = request.request_id;
        let fail = |e: Error| BrokerResponse::failure(id, e);
        if let Err(e) = validate_request(request) {
            return fail(e);
        }
        let Some(op) = self.registry.get(&request.operation) else {
            return fail(
                Error::new(ErrorCode::NotFound, "Unknown operation.")
                    .with_user_text(texts::unknown_operation(&request.operation)),
            );
        };
        let d = op.descriptor().clone();
        let decision = self.policy.authorize(client, d.tier);
        if !decision.allowed {
            log::warn!(target: AUDIT, "Denied {} for client [{client}]: {}", d.name, decision.reason);
            let text = if decision.elevation_would_help {
                texts::elevation_required()
            } else {
                texts::client_not_authorized()
            };
            return fail(Error::new(ErrorCode::AccessDenied, "Not authorized.").with_user_text(text));
        }
        let canonical = match op.bind(&request.parameters) {
            Ok(c) => c,
            Err(e) => {
                let text = e.user_text.clone().unwrap_or_else(texts::malformed_request);
                return fail(e.with_user_text(text));
            }
        };
        *audit_parameters = op.audit(&canonical, self.options.audit_parameter_chars);
        if let Err(e) = op.validate(&canonical) {
            return fail(e);
        }
        let timeout = d.timeout.unwrap_or(self.options.default_timeout);
        let now = OffsetDateTime::now_utc();
        let mut context = CallContext {
            request_id: id,
            operation: d.name.to_owned(),
            client: client.clone(),
            is_confirmed: false,
            received_at: now,
            deadline: Instant::now() + timeout,
            connection_closed,
        };
        if d.is_mutating() || d.requires_confirmation {
            let plan = match op.plan(&canonical, &context) {
                Ok(p) => p,
                Err(e) => return fail(e),
            };
            if plan.is_none() && d.requires_confirmation {
                return fail(
                    Error::new(
                        ErrorCode::PreconditionFailed,
                        format!("Operation '{}' requires confirmation but produced no plan.", d.name),
                    )
                    .with_user_text(texts::internal()),
                );
            }
            if let Some(mut plan) = plan {
                plan.operation = d.name.to_owned();
                if let Err(e) = plan.validate() {
                    return fail(e.with_user_text(texts::internal()));
                }
                if d.requires_confirmation || plan.requires_explicit_confirmation() {
                    match &request.confirmation_token {
                        None => {
                            return match self.tokens.issue(d.name, &canonical, client, &plan, now) {
                                Ok((token, expires)) => BrokerResponse::needs_confirmation(id, plan, token, expires),
                                Err(_) => fail(
                                    Error::new(ErrorCode::PlatformError, "No random source.")
                                        .with_user_text(texts::internal()),
                                ),
                            };
                        }
                        Some(token) if !self.tokens.consume(token, d.name, &canonical, client, &plan, now) => {
                            return fail(
                                Error::new(
                                    ErrorCode::ConfirmationRequired,
                                    "Confirmation token invalid, expired, reused or the plan changed.",
                                )
                                .with_user_text(texts::confirmation_invalid()),
                            );
                        }
                        Some(_) => context.is_confirmed = true,
                    }
                }
            }
        }
        let Some(_lease) = self.gate.try_enter(&d.category, self.options.busy_wait) else {
            return fail(Error::new(ErrorCode::Busy, "Category busy.").with_user_text(texts::busy(d.category.name())));
        };
        match op.execute(&canonical, &context) {
            Ok(result) => BrokerResponse::ok(id, result),
            Err(e) if e.code == ErrorCode::Timeout && e.user_text.is_none() => fail(e.with_user_text(texts::timeout())),
            Err(e) => fail(e),
        }
    }
}
