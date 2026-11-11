//! Serves one client connection: reads frames, dispatches up to 8 requests concurrently, writes responses as
//! they finish (correlated by request id). A framing error or a malformed envelope closes the connection. The
//! client identity is resolved once, after the first frame (the pipe client PID is known only then).

use crate::dispatch::Dispatcher;
use crate::envelope::{BrokerRequest, BrokerResponse, texts};
use crate::framing::{FrameRead, read_frame, write_frame};
use crate::identity::ClientIdentity;
use crate::protocol::{MAX_FRAME_BYTES, MAX_IN_FLIGHT_PER_CONNECTION};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use uuid::Uuid;

struct Slots {
    free: Mutex<usize>,
    released: Condvar,
}

impl Slots {
    fn acquire(&self) {
        if let Ok(mut free) = self.free.lock() {
            while *free == 0 {
                match self.released.wait(free) {
                    Ok(f) => free = f,
                    Err(_) => return,
                }
            }
            *free -= 1;
        }
    }
    fn release(&self) {
        if let Ok(mut free) = self.free.lock() {
            *free += 1;
        }
        self.released.notify_one();
    }
}

fn send(writer: &Mutex<impl Write>, response: &BrokerResponse) -> bool {
    let Ok(bytes) = serde_json::to_vec(response) else { return false };
    writer.lock().is_ok_and(|mut w| write_frame(&mut *w, &bytes, MAX_FRAME_BYTES).is_ok())
}

/// Runs until the client disconnects or misbehaves; waits for in-flight requests before returning.
pub fn serve<R: Read, W: Write + Send>(
    dispatcher: &Dispatcher,
    mut reader: R,
    writer: W,
    resolve_identity: impl FnOnce() -> ClientIdentity,
) {
    let writer = Mutex::new(writer);
    let closed = Arc::new(AtomicBool::new(false));
    let slots = Slots { free: Mutex::new(MAX_IN_FLIGHT_PER_CONNECTION), released: Condvar::new() };
    let mut resolve = Some(resolve_identity);
    let mut client: Option<ClientIdentity> = None;
    std::thread::scope(|scope| {
        loop {
            let payload = match read_frame(&mut reader, MAX_FRAME_BYTES) {
                FrameRead::Frame(p) => p,
                FrameRead::EndOfStream => break,
                FrameRead::Failed(why) => {
                    log::warn!("Closing broker connection: {why:?}");
                    break;
                }
            };
            if client.is_none() {
                client = resolve.take().map(|f| f());
            }
            let identity = client.clone().unwrap_or_else(|| ClientIdentity::unresolved("identity not resolved", 0));
            let request = crate::json::parse(&payload).and_then(|v| Ok(serde_json::from_value::<BrokerRequest>(v)?));
            let request = match request {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("Closing broker connection after malformed request from [{identity}]: {e}");
                    send(&writer, &BrokerResponse::failure(Uuid::nil(), e.with_user_text(texts::malformed_request())));
                    break;
                }
            };
            slots.acquire();
            let (writer, closed, slots) = (&writer, closed.clone(), &slots);
            scope.spawn(move || {
                let response = dispatcher.dispatch(&request, &identity, closed);
                if !send(writer, &response) {
                    log::debug!("Could not deliver a broker response (client gone)");
                }
                slots.release();
            });
        }
        closed.store(true, Ordering::Relaxed);
    });
}
