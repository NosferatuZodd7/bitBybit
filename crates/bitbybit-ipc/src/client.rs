//! Broker client used by the app and the CLI. Several requests may be in flight; a reader thread routes responses
//! by request id. Confirmation is explicit: when the broker returns a plan, `confirm` shows it to the user and
//! only an explicit yes re-sends the identical request with the token.

use crate::envelope::{BrokerRequest, BrokerResponse, ResponseStatus, validate_response};
use crate::framing::{FrameRead, read_frame, write_frame};
use crate::plan::OperationPlan;
use crate::protocol::{MAX_FRAME_BYTES, PROTOCOL_VERSION};
use bitbybit_core::{Error, ErrorCode, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::OffsetDateTime;
use uuid::Uuid;

/// Shows the plan to the user; true only on an explicit yes.
pub type ConfirmPlan<'a> = &'a dyn Fn(&OperationPlan, Option<OffsetDateTime>) -> bool;

type Pending = Arc<Mutex<HashMap<Uuid, Sender<BrokerResponse>>>>;

pub struct BrokerClient<W: Write> {
    writer: Mutex<W>,
    pending: Pending,
    closed: Arc<AtomicBool>,
}

impl<W: Write> BrokerClient<W> {
    /// Starts the reader thread on `reader`.
    pub fn new<R: Read + Send + 'static>(mut reader: R, writer: W) -> Self {
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (p, c) = (pending.clone(), closed.clone());
        std::thread::spawn(move || {
            while let FrameRead::Frame(payload) = read_frame(&mut reader, MAX_FRAME_BYTES) {
                let Ok(response) =
                    crate::json::parse(&payload).and_then(|v| Ok(serde_json::from_value::<BrokerResponse>(v)?))
                else {
                    break;
                };
                if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&response.request_id)) {
                    let _ = tx.send(response);
                }
            }
            c.store(true, Ordering::Relaxed);
            if let Ok(mut m) = p.lock() {
                m.clear(); // waiting callers see a disconnected channel
            }
        });
        Self { writer: Mutex::new(writer), pending, closed }
    }

    pub fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::Relaxed)
    }

    fn send(
        &self,
        operation: &str,
        parameters: &Value,
        token: Option<String>,
        timeout: Duration,
    ) -> Result<BrokerResponse> {
        if !self.is_connected() {
            return Err(Error::new(ErrorCode::PlatformError, "The broker connection is closed."));
        }
        let request = BrokerRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            operation: operation.to_owned(),
            parameters: parameters.clone(),
            confirmation_token: token,
        };
        let (tx, rx) = mpsc::channel();
        self.pending.lock().map_err(|_| Error::invalid("client poisoned"))?.insert(request.request_id, tx);
        let bytes = serde_json::to_vec(&request)?;
        let written = self
            .writer
            .lock()
            .map_err(|_| Error::invalid("client poisoned"))
            .and_then(|mut w| write_frame(&mut *w, &bytes, MAX_FRAME_BYTES));
        let response = written.and_then(|()| match rx.recv_timeout(timeout) {
            Ok(r) => Ok(r),
            Err(RecvTimeoutError::Timeout) => Err(Error::new(ErrorCode::Timeout, "The broker did not answer in time.")),
            Err(RecvTimeoutError::Disconnected) => {
                Err(Error::new(ErrorCode::PlatformError, "The broker connection closed."))
            }
        });
        if let Ok(mut m) = self.pending.lock() {
            m.remove(&request.request_id);
        }
        let response = response?;
        validate_response(&response)?;
        Ok(response)
    }

    /// Calls an operation. `confirm` receives the plan when the broker asks for confirmation; returning false
    /// cancels. Without `confirm`, operations that need confirmation fail with `ConfirmationRequired`.
    pub fn invoke<Req: Serialize, Resp: DeserializeOwned>(
        &self,
        operation: &str,
        request: &Req,
        confirm: Option<ConfirmPlan<'_>>,
        timeout: Duration,
    ) -> Result<Resp> {
        let parameters = serde_json::to_value(request)?;
        let mut r = self.send(operation, &parameters, None, timeout)?;
        if r.status == ResponseStatus::ConfirmationRequired {
            let (Some(plan), Some(token)) = (r.plan.as_ref(), r.confirmation_token.clone()) else {
                return Err(Error::new(ErrorCode::VerificationFailed, "Inconsistent confirmation response."));
            };
            let Some(confirm) = confirm else {
                return Err(Error::new(ErrorCode::ConfirmationRequired, "The operation needs explicit confirmation."));
            };
            if !confirm(plan, r.confirmation_expires_at) {
                return Err(Error::new(ErrorCode::Cancelled, "The user declined the operation."));
            }
            r = self.send(operation, &parameters, Some(token), timeout)?;
        }
        match (r.status, r.result, r.error) {
            (ResponseStatus::Ok, Some(result), _) => Ok(serde_json::from_value(result)?),
            (ResponseStatus::Error, _, Some(e)) => Err(e),
            _ => Err(Error::new(ErrorCode::VerificationFailed, "The broker asked for confirmation twice.")),
        }
    }
}
