//! Request/response envelopes and their semantic checks (shared by broker and client).

use crate::plan::OperationPlan;
use crate::protocol::PROTOCOL_VERSION;
use bitbybit_core::{Error, ErrorCode, LocalizedText, Result, text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

pub const MAX_OPERATION_NAME: usize = 96;
pub const MAX_TOKEN_LENGTH: usize = 128;

pub mod texts {
    use super::*;
    pub fn unsupported_protocol(version: u32) -> LocalizedText {
        text!(
            "ipc.error.unsupportedProtocol",
            "The bitBybit service uses a different protocol version ({0}). Reinstall or update bitBybit.",
            version.to_string()
        )
    }
    pub fn malformed_request() -> LocalizedText {
        text!("ipc.error.malformedRequest", "The request sent to the bitBybit service is not valid.")
    }
    pub fn confirmation_invalid() -> LocalizedText {
        text!(
            "ipc.error.confirmationInvalid",
            "The confirmation is no longer valid. Review the operation and confirm again."
        )
    }
    pub fn unknown_operation(name: &str) -> LocalizedText {
        text!("ipc.error.unknownOperation", "The bitBybit service does not allow the operation '{0}'.", name.to_owned())
    }
    pub fn client_not_authorized() -> LocalizedText {
        text!(
            "ipc.error.clientNotAuthorized",
            "This program is not an installed bitBybit component and cannot use the bitBybit service."
        )
    }
    pub fn elevation_required() -> LocalizedText {
        text!(
            "ipc.error.elevationRequired",
            "This operation requires administrator rights. bitBybit will ask Windows for permission (UAC)."
        )
    }
    pub fn busy(category: &str) -> LocalizedText {
        text!("ipc.error.busy", "Another '{0}' operation is in progress. Try again in a moment.", category.to_owned())
    }
    pub fn timeout() -> LocalizedText {
        text!("ipc.error.timeout", "The operation took too long and was stopped.")
    }
    pub fn internal() -> LocalizedText {
        text!(
            "ipc.error.internal",
            "The bitBybit service hit an unexpected error. Details were written to the service log."
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerRequest {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub operation: String,
    /// Always a JSON object, bound to the operation's request type.
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseStatus {
    Ok,
    Error,
    /// `plan` and `confirmationToken` are set; re-send the identical request with the token.
    ConfirmationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerResponse {
    pub request_id: Uuid,
    pub status: ResponseStatus,
    /// Never contains stack traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<OperationPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_token: Option<String>,
    /// Display only; the broker checks expiry itself.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "time::serde::rfc3339::option")]
    pub confirmation_expires_at: Option<OffsetDateTime>,
}

impl BrokerResponse {
    fn empty(request_id: Uuid, status: ResponseStatus) -> Self {
        Self {
            request_id,
            status,
            error: None,
            result: None,
            plan: None,
            confirmation_token: None,
            confirmation_expires_at: None,
        }
    }
    pub fn ok(request_id: Uuid, result: Value) -> Self {
        Self { result: Some(result), ..Self::empty(request_id, ResponseStatus::Ok) }
    }
    pub fn failure(request_id: Uuid, error: Error) -> Self {
        Self { error: Some(error), ..Self::empty(request_id, ResponseStatus::Error) }
    }
    pub fn needs_confirmation(
        request_id: Uuid,
        plan: OperationPlan,
        token: String,
        expires_at: OffsetDateTime,
    ) -> Self {
        Self {
            plan: Some(plan),
            confirmation_token: Some(token),
            confirmation_expires_at: Some(expires_at),
            ..Self::empty(request_id, ResponseStatus::ConfirmationRequired)
        }
    }
}

/// `module.verb[.more]`, e.g. `broker.ping`, `audio.setDefaultEndpoint`.
pub fn is_valid_operation_name(name: &str) -> bool {
    let mut parts = name.split('.');
    let first_ok = parts.next().is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()));
    let rest: Vec<&str> = parts.collect();
    name.len() <= MAX_OPERATION_NAME
        && first_ok
        && !rest.is_empty()
        && rest.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Valid names verbatim; anything else summarized, never echoed raw into logs.
pub fn sanitize_operation_name(name: &str) -> String {
    if is_valid_operation_name(name) {
        name.to_owned()
    } else {
        format!("<invalid operation name, {} chars>", name.chars().count())
    }
}

fn malformed(message: &str) -> Error {
    Error::invalid(message).with_user_text(texts::malformed_request())
}

pub fn validate_request(r: &BrokerRequest) -> Result<()> {
    if r.protocol_version != PROTOCOL_VERSION {
        return Err(Error::new(
            ErrorCode::NotSupported,
            format!("Unsupported protocol version {}; expected {PROTOCOL_VERSION}.", r.protocol_version),
        )
        .with_user_text(texts::unsupported_protocol(r.protocol_version)));
    }
    if r.request_id.is_nil() {
        return Err(malformed("requestId must not be empty."));
    }
    if !is_valid_operation_name(&r.operation) {
        return Err(malformed("Operation name is not valid."));
    }
    if !r.parameters.is_object() {
        return Err(malformed("parameters must be a JSON object."));
    }
    if let Some(t) = &r.confirmation_token
        && (t.is_empty()
            || t.len() > MAX_TOKEN_LENGTH
            || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    {
        return Err(Error::invalid("confirmationToken is malformed.").with_user_text(texts::confirmation_invalid()));
    }
    Ok(())
}

/// The client rejects internally inconsistent responses.
pub fn validate_response(r: &BrokerResponse) -> Result<()> {
    let consistent = match r.status {
        ResponseStatus::Ok => {
            r.error.is_none()
                && r.result.as_ref().is_some_and(Value::is_object)
                && r.plan.is_none()
                && r.confirmation_token.is_none()
        }
        ResponseStatus::Error => r.error.is_some() && r.result.is_none() && r.confirmation_token.is_none(),
        ResponseStatus::ConfirmationRequired => {
            r.error.is_none()
                && r.result.is_none()
                && r.plan.is_some()
                && r.confirmation_token.as_deref().is_some_and(|t| !t.is_empty())
        }
    };
    if consistent {
        Ok(())
    } else {
        Err(Error::new(ErrorCode::VerificationFailed, format!("Inconsistent broker response (status {:?}).", r.status)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_names() {
        for good in ["broker.ping", "audio.setDefaultEndpoint", "broker.events.poll"] {
            assert!(is_valid_operation_name(good), "{good}");
        }
        for bad in [
            "ping",
            "Broker.ping",
            "broker.",
            ".ping",
            "broker..ping",
            "broker.pi ng",
            "broker.pi-ng",
            &format!("a.{}", "b".repeat(96)),
        ] {
            assert!(!is_valid_operation_name(bad), "{bad}");
        }
        assert_eq!(sanitize_operation_name("rm -rf /"), "<invalid operation name, 8 chars>");
    }

    #[test]
    fn request_checks() {
        let ok = BrokerRequest {
            protocol_version: 1,
            request_id: Uuid::new_v4(),
            operation: "broker.ping".into(),
            parameters: serde_json::json!({}),
            confirmation_token: None,
        };
        assert!(validate_request(&ok).is_ok());
        assert_eq!(
            validate_request(&BrokerRequest { protocol_version: 2, ..ok.clone() }).unwrap_err().code,
            ErrorCode::NotSupported
        );
        assert!(validate_request(&BrokerRequest { request_id: Uuid::nil(), ..ok.clone() }).is_err());
        assert!(validate_request(&BrokerRequest { parameters: serde_json::json!([]), ..ok.clone() }).is_err());
        assert!(validate_request(&BrokerRequest { confirmation_token: Some("a+b".into()), ..ok.clone() }).is_err());
        let json = serde_json::to_string(&ok).unwrap().replace('}', ",\"extra\":1}");
        assert!(serde_json::from_str::<BrokerRequest>(&json).is_err(), "unknown envelope fields are rejected");
    }

    #[test]
    fn response_consistency() {
        let id = Uuid::new_v4();
        assert!(validate_response(&BrokerResponse::ok(id, serde_json::json!({}))).is_ok());
        assert!(validate_response(&BrokerResponse::ok(id, serde_json::json!(5))).is_err());
        assert!(validate_response(&BrokerResponse::failure(id, Error::invalid("x"))).is_ok());
        let mut mixed = BrokerResponse::ok(id, serde_json::json!({}));
        mixed.error = Some(Error::invalid("x"));
        assert!(validate_response(&mixed).is_err());
    }
}
