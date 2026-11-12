//! Named-pipe transport for the broker protocol. Overlapped I/O on every handle: synchronous I/O on one pipe
//! handle is serialized by Windows, so a response write would block behind the pending request read. Overlapped
//! waits also give real timeouts (slow-loris guard) through `CancelIoEx`.
//!
//! Server: DACL grants SYSTEM and Administrators full access, interactive users read/write, denies network
//! logons; remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`); the first instance is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE` so another process cannot squat the name. Client identity comes from OS facts
//! (client PID → image path and token). Client: connects with `SECURITY_IDENTIFICATION` (the server cannot act
//! as the user) and can check that the server runs as LocalSystem.

use bitbybit_ipc::identity::{ClientIdentity, IntegrityLevel};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
    HLOCAL, LocalFree, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenElevation,
    TokenIntegrityLevel, TokenSessionId, TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows::Win32::System::Threading::{
    CreateEventW, INFINITE, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::core::{HSTRING, PWSTR};

/// SYSTEM + Administrators: all; interactive users: read/write; network logons: denied.
pub const PIPE_SDDL: &str = "D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";
const BUFFER: u32 = 64 * 1024;
/// Clients poll events at least every 25 s, so a connection silent this long is dead or hostile.
pub const SERVER_READ_TIMEOUT: Duration = Duration::from_secs(300);

/// An owned kernel handle.
struct Owned(HANDLE);
// SAFETY: kernel handles may be used from any thread; overlapped I/O keeps concurrent operations independent.
unsafe impl Send for Owned {}
// SAFETY: see above.
unsafe impl Sync for Owned {}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: owned handle, closed once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

fn io_err(e: &windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

/// One duplex pipe connection. Clones share the handle; each read/write uses its own OVERLAPPED.
#[derive(Clone)]
pub struct PipeStream {
    handle: Arc<Owned>,
    /// Applied to each read once a frame started; `None` waits forever (idle connection).
    pub read_timeout: Option<Duration>,
    pub write_timeout: Option<Duration>,
}

impl PipeStream {
    fn raw(&self) -> HANDLE {
        self.handle.0
    }

    /// Runs one overlapped operation to completion or timeout.
    fn overlapped(
        &self,
        timeout: Option<Duration>,
        start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
    ) -> io::Result<u32> {
        // SAFETY: manual-reset, unnamed event; owned below.
        let event = Owned(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| io_err(&e))?);
        let mut ov = OVERLAPPED { hEvent: event.0, ..Default::default() };
        let pending = match start(&mut ov) {
            Ok(()) => false,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => true,
            Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() || e.code() == ERROR_NO_DATA.to_hresult() => {
                return Ok(0);
            }
            Err(e) => return Err(io_err(&e)),
        };
        let mut n = 0u32;
        if pending {
            let ms = timeout.map_or(INFINITE, |t| u32::try_from(t.as_millis()).unwrap_or(INFINITE - 1));
            // SAFETY: waits on our own event.
            if unsafe { WaitForSingleObject(event.0, ms) } != WAIT_OBJECT_0 {
                // SAFETY: cancels only this operation, then waits until the kernel no longer touches `ov`/the buffer.
                unsafe {
                    let _ = CancelIoEx(self.raw(), Some(&ov));
                    let _ = GetOverlappedResult(self.raw(), &ov, &mut n, true);
                }
                return Err(io::ErrorKind::TimedOut.into());
            }
        }
        // SAFETY: the operation has completed; `ov` is still alive.
        match unsafe { GetOverlappedResult(self.raw(), &ov, &mut n, false) } {
            Ok(()) => Ok(n),
            Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() || e.code() == ERROR_NO_DATA.to_hresult() => Ok(0),
            Err(e) => Err(io_err(&e)),
        }
    }
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(BUFFER as usize);
        let ptr = buf.as_mut_ptr();
        let n = self.overlapped(self.read_timeout, |ov| {
            // SAFETY: `buf` outlives the operation (overlapped() waits for completion or cancellation).
            unsafe { ReadFile(self.raw(), Some(std::slice::from_raw_parts_mut(ptr, len)), None, Some(ov)) }
        })?;
        Ok(n as usize)
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(BUFFER as usize);
        let n = self.overlapped(self.write_timeout, |ov| {
            // SAFETY: `buf` outlives the operation (overlapped() waits for completion or cancellation).
            unsafe { WriteFile(self.raw(), Some(&buf[..len]), None, Some(ov)) }
        })?;
        if n == 0 && len > 0 { Err(io::ErrorKind::BrokenPipe.into()) } else { Ok(n as usize) }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // byte-mode pipe writes are delivered on completion
    }
}

fn pipe_path(name: &str) -> HSTRING {
    HSTRING::from(format!(r"\\.\pipe\{name}"))
}

/// Frees a LocalAlloc'd security descriptor.
struct LocalSd(PSECURITY_DESCRIPTOR);
impl Drop for LocalSd {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW, freed once.
        unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
    }
}

/// Listens on `\\.\pipe\{name}`; one instance per pending client.
pub struct PipeServer {
    name: String,
    sd: LocalSd,
    first: std::sync::atomic::AtomicBool,
}

// SAFETY: the security descriptor is immutable after creation and only read by CreateNamedPipeW.
unsafe impl Send for PipeServer {}
// SAFETY: see above.
unsafe impl Sync for PipeServer {}

impl PipeServer {
    pub fn new(name: &str) -> io::Result<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: constant SDDL; output freed by LocalSd.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(PIPE_SDDL),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|e| io_err(&e))?;
        Ok(Self { name: name.to_owned(), sd: LocalSd(sd), first: std::sync::atomic::AtomicBool::new(true) })
    }

    /// Blocks until a client connects; returns its stream.
    pub fn accept(&self) -> io::Result<PipeStream> {
        let first = self.first.swap(false, std::sync::atomic::Ordering::SeqCst);
        let mut mode: FILE_FLAGS_AND_ATTRIBUTES = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.sd.0.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: valid name and security attributes for the duration of the call.
        let handle = unsafe {
            CreateNamedPipeW(
                &pipe_path(&self.name),
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                Some(&sa),
            )
        };
        if handle.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        // shortcut: one timeout covers idle and mid-frame reads (5 min); split them if slow-loris becomes a concern.
        let stream = PipeStream {
            handle: Arc::new(Owned(handle)),
            read_timeout: Some(SERVER_READ_TIMEOUT),
            write_timeout: Some(Duration::from_secs(10)),
        };
        match stream.overlapped(None, |ov| {
            // SAFETY: owned pipe handle; `ov` lives until completion.
            unsafe { ConnectNamedPipe(stream.raw(), Some(ov)) }
        }) {
            Ok(_) => Ok(stream),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED.0 as i32) => Ok(stream),
            Err(e) => Err(e),
        }
    }
}

/// Connects to `\\.\pipe\{name}`, waiting up to `timeout` while all instances are busy.
pub fn connect(name: &str, timeout: Duration) -> io::Result<PipeStream> {
    let path = pipe_path(name);
    let deadline = Instant::now() + timeout;
    loop {
        // SAFETY: valid path; the returned handle is owned.
        let opened = unsafe {
            CreateFileW(
                &path,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match opened {
            Ok(h) => {
                return Ok(PipeStream {
                    handle: Arc::new(Owned(h)),
                    read_timeout: Some(timeout.max(Duration::from_secs(30))),
                    write_timeout: Some(Duration::from_secs(10)),
                });
            }
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() && Instant::now() < deadline => {
                let left = deadline.saturating_duration_since(Instant::now()).as_millis();
                // SAFETY: valid path.
                let _ = unsafe { WaitNamedPipeW(&path, u32::try_from(left).unwrap_or(1000)) };
            }
            Err(e) => return Err(io_err(&e)),
        }
    }
}

/// `GetTokenInformation` into an 8-byte aligned buffer.
fn token_info(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    let mut needed = 0u32;
    // SAFETY: size query with no buffer.
    let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut needed) };
    let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    // SAFETY: the buffer holds at least `needed` bytes.
    unsafe { GetTokenInformation(token, class, Some(buf.as_mut_ptr().cast()), needed, &mut needed) }.ok()?;
    Some(buf)
}

fn sid_string(sid: PSID) -> Option<String> {
    let mut s = PWSTR::null();
    // SAFETY: valid SID from the token buffer; the string is LocalFree'd after copying.
    unsafe {
        ConvertSidToStringSidW(sid, &mut s).ok()?;
        let out = s.to_string().ok();
        LocalFree(Some(HLOCAL(s.0.cast())));
        out
    }
}

struct ProcessFacts {
    image: Option<String>,
    sid: Option<String>,
    elevated: bool,
    integrity: IntegrityLevel,
    session: Option<u32>,
}

fn process_facts(pid: u32) -> io::Result<ProcessFacts> {
    // SAFETY: query-only access; handle owned below.
    let process = Owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map_err(|e| io_err(&e))?);
    let mut path = vec![0u16; 32_768];
    let mut len = path.len() as u32;
    // SAFETY: buffer of `len` UTF-16 units.
    let image =
        unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len) }
            .ok()
            .map(|()| String::from_utf16_lossy(&path[..len as usize]));
    let mut token = HANDLE::default();
    // SAFETY: query-only token access; owned below.
    unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) }.map_err(|e| io_err(&e))?;
    let token = Owned(token);
    // SAFETY (all reads below): each buffer was filled by GetTokenInformation for the matching class.
    let sid =
        token_info(token.0, TokenUser).and_then(|b| sid_string(unsafe { (*b.as_ptr().cast::<TOKEN_USER>()).User.Sid }));
    let elevated = token_info(token.0, TokenElevation)
        .is_some_and(|b| unsafe { (*b.as_ptr().cast::<TOKEN_ELEVATION>()).TokenIsElevated } != 0);
    let integrity = token_info(token.0, TokenIntegrityLevel)
        .map(|b| unsafe {
            let sid = (*b.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            IntegrityLevel::from_rid(*GetSidSubAuthority(sid, u32::from(count.saturating_sub(1))))
        })
        .unwrap_or(IntegrityLevel::Untrusted);
    let session = token_info(token.0, TokenSessionId).map(|b| b[0] as u32);
    Ok(ProcessFacts { image, sid, elevated, integrity, session })
}

/// Whether this process runs elevated (false when the token cannot be read).
pub fn current_process_is_elevated() -> bool {
    process_facts(std::process::id()).is_ok_and(|f| f.elevated)
}

/// Who is connected (from the pipe's client PID). Never fails: problems yield an unresolved identity.
pub fn client_identity(stream: &PipeStream) -> ClientIdentity {
    let mut pid = 0u32;
    // SAFETY: connected server-side pipe handle.
    if let Err(e) = unsafe { GetNamedPipeClientProcessId(stream.raw(), &mut pid) } {
        return ClientIdentity::unresolved(&format!("GetNamedPipeClientProcessId failed: {e}"), 0);
    }
    identity_of(pid)
}

/// This process, for a broker hosted in-process by the elevated app (no pipe involved).
pub fn own_identity() -> ClientIdentity {
    identity_of(std::process::id())
}

fn identity_of(pid: u32) -> ClientIdentity {
    match process_facts(pid) {
        Ok(f) => ClientIdentity {
            process_id: pid,
            image_path: f.image,
            user_sid: f.sid,
            is_elevated: f.elevated,
            integrity: f.integrity,
            session_id: f.session,
            // PIPE_REJECT_REMOTE_CLIENTS and the NU deny ACE keep remote and network-logon callers out.
            is_local: true,
            resolution_error: None,
        },
        Err(e) => ClientIdentity::unresolved(&format!("client process could not be inspected: {e}"), pid),
    }
}

/// Client side: true when the pipe server process runs as LocalSystem (protects against a squatted pipe name).
pub fn server_is_local_system(stream: &PipeStream) -> bool {
    let mut pid = 0u32;
    // SAFETY: connected client-side pipe handle.
    unsafe { GetNamedPipeServerProcessId(stream.raw(), &mut pid) }.is_ok()
        && process_facts(pid).is_ok_and(|f| f.sid.as_deref() == Some(bitbybit_ipc::protocol::LOCAL_SYSTEM_SID))
}
