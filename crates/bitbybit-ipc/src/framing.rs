//! Wire framing: a 4-byte little-endian payload length followed by that many bytes of UTF-8 JSON. Zero-length and
//! oversized frames are rejected before the payload is buffered. Partial-read safe. Timeouts come from the
//! transport (the named-pipe wrapper returns `TimedOut`/`WouldBlock`); after any failure the connection must be
//! closed because the stream may be desynchronized.

use crate::protocol::MAX_FRAME_BYTES;
use bitbybit_core::{Error, ErrorCode, Result};
use std::io::{ErrorKind, Read, Write};

pub const HEADER_SIZE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFailure {
    IdleTimeout,
    FrameTimeout,
    ZeroLength,
    TooLarge,
    TruncatedHeader,
    TruncatedPayload,
    Io,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameRead {
    Frame(Vec<u8>),
    /// Clean close at a frame boundary.
    EndOfStream,
    Failed(FrameFailure),
}

fn is_timeout(kind: ErrorKind) -> bool {
    matches!(kind, ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

/// Fills `buf`; `Ok(false)` when the stream ended first.
fn read_exact_or_eof(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut at = 0;
    while at < buf.len() {
        match r.read(&mut buf[at..]) {
            Ok(0) => return Ok(false),
            Ok(n) => at += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

pub fn read_frame(r: &mut impl Read, max_bytes: usize) -> FrameRead {
    let max = max_bytes.clamp(1, MAX_FRAME_BYTES);
    let mut header = [0u8; HEADER_SIZE];
    // First byte: the connection may legitimately be idle for a long time.
    loop {
        match r.read(&mut header[..1]) {
            Ok(0) => return FrameRead::EndOfStream,
            Ok(_) => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if is_timeout(e.kind()) => return FrameRead::Failed(FrameFailure::IdleTimeout),
            Err(_) => return FrameRead::Failed(FrameFailure::Io),
        }
    }
    let fail_io = |e: std::io::Error| {
        FrameRead::Failed(if is_timeout(e.kind()) { FrameFailure::FrameTimeout } else { FrameFailure::Io })
    };
    match read_exact_or_eof(r, &mut header[1..]) {
        Ok(true) => {}
        Ok(false) => return FrameRead::Failed(FrameFailure::TruncatedHeader),
        Err(e) => return fail_io(e),
    }
    let length = u32::from_le_bytes(header) as usize;
    if length == 0 {
        return FrameRead::Failed(FrameFailure::ZeroLength);
    }
    if length > max {
        return FrameRead::Failed(FrameFailure::TooLarge);
    }
    let mut payload = vec![0u8; length];
    match read_exact_or_eof(r, &mut payload) {
        Ok(true) => FrameRead::Frame(payload),
        Ok(false) => FrameRead::Failed(FrameFailure::TruncatedPayload),
        Err(e) => fail_io(e),
    }
}

/// Header + payload as one buffer.
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(HEADER_SIZE + payload.len());
    buffer.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buffer.extend_from_slice(payload);
    buffer
}

/// Writes one frame in a single write and flushes.
pub fn write_frame(w: &mut impl Write, payload: &[u8], max_bytes: usize) -> Result<()> {
    if payload.is_empty() {
        return Err(Error::invalid("Refusing to write a zero-length frame."));
    }
    if payload.len() > max_bytes.clamp(1, MAX_FRAME_BYTES) {
        return Err(Error::invalid(format!("Payload of {} bytes exceeds the frame limit.", payload.len())));
    }
    let result = w.write_all(&encode(payload)).and_then(|()| w.flush());
    result.map_err(|e| {
        Error::new(
            if is_timeout(e.kind()) { ErrorCode::Timeout } else { ErrorCode::PlatformError },
            format!("Frame write failed: {:?}", e.kind()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Delivers at most 3 bytes per read, like a pipe under load.
    struct Trickle(Cursor<Vec<u8>>);
    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(3);
            self.0.read(&mut buf[..n])
        }
    }

    #[test]
    fn round_trips_frames_with_partial_reads() {
        let mut wire = Vec::new();
        write_frame(&mut wire, b"{\"a\":1}", MAX_FRAME_BYTES).unwrap();
        write_frame(&mut wire, &vec![b'x'; 5000], MAX_FRAME_BYTES).unwrap();
        let mut r = Trickle(Cursor::new(wire));
        assert_eq!(read_frame(&mut r, MAX_FRAME_BYTES), FrameRead::Frame(b"{\"a\":1}".to_vec()));
        assert!(matches!(read_frame(&mut r, MAX_FRAME_BYTES), FrameRead::Frame(p) if p.len() == 5000));
        assert_eq!(read_frame(&mut r, MAX_FRAME_BYTES), FrameRead::EndOfStream);
    }

    #[test]
    fn rejects_zero_length_oversize_and_truncated_frames() {
        let read = |bytes: Vec<u8>, max| read_frame(&mut Cursor::new(bytes), max);
        assert_eq!(read(vec![0, 0, 0, 0], 100), FrameRead::Failed(FrameFailure::ZeroLength));
        assert_eq!(read(encode(&[1; 101]), 100), FrameRead::Failed(FrameFailure::TooLarge));
        assert_eq!(
            read((MAX_FRAME_BYTES as u32 + 1).to_le_bytes().to_vec(), usize::MAX),
            FrameRead::Failed(FrameFailure::TooLarge)
        );
        assert_eq!(read(vec![5, 0], 100), FrameRead::Failed(FrameFailure::TruncatedHeader));
        assert_eq!(read(vec![5, 0, 0, 0, 1, 2], 100), FrameRead::Failed(FrameFailure::TruncatedPayload));
        assert!(write_frame(&mut Vec::new(), b"", 100).is_err());
        assert!(write_frame(&mut Vec::new(), &[1; 101], 100).is_err());
    }

    #[test]
    fn timeouts_are_classified_by_phase() {
        struct TimesOut(usize);
        impl Read for TimesOut {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    return Err(ErrorKind::TimedOut.into());
                }
                self.0 -= 1;
                buf[0] = 9;
                Ok(1)
            }
        }
        assert_eq!(read_frame(&mut TimesOut(0), 100), FrameRead::Failed(FrameFailure::IdleTimeout));
        assert_eq!(read_frame(&mut TimesOut(2), 100), FrameRead::Failed(FrameFailure::FrameTimeout), "slow-loris");
    }
}
