//! Starts one of the allowlisted System32 tools (`bitbybit_security::tools::FixedTool`) with validated arguments:
//! absolute System32 path (never PATH or the working directory), an argument list (no shell, no command string),
//! no window, captured output, bounded run time.

use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_security::tools::{FixedTool, ToolOutput, ToolRunner, validate_arguments};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_CAPTURED_BYTES: u64 = 4 * 1024 * 1024;

pub fn system32() -> Result<PathBuf> {
    let mut buf = [0u16; 260];
    // SAFETY: buffer of the given length; returns the number of characters written.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    if len == 0 || len > buf.len() {
        return Err(Error::platform("GetSystemDirectoryW failed", 0));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

pub fn resolve(tool: FixedTool) -> Result<PathBuf> {
    let path = system32()?.join(tool.file_name());
    if path.is_file() { Ok(path) } else { Err(Error::not_found(format!("System tool not found: {}", path.display()))) }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemToolRunner;

impl ToolRunner for SystemToolRunner {
    fn run(&self, tool: FixedTool, arguments: &[String], timeout: Duration) -> Result<ToolOutput> {
        validate_arguments(arguments)?;
        let path = resolve(tool)?;
        let started = Instant::now();
        let mut child = Command::new(&path)
            .args(arguments)
            .current_dir(path.parent().unwrap_or(&path))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        let capture = |stream: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(s) = stream {
                    let _ = s.take(MAX_CAPTURED_BYTES).read_to_end(&mut bytes);
                }
                String::from_utf8_lossy(&bytes).into_owned()
            })
        };
        let stdout = capture(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
        let stderr = capture(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
        let status = loop {
            match child.try_wait()? {
                Some(status) => break status,
                None if started.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::new(
                        ErrorCode::Timeout,
                        format!("{} timed out after {}s.", tool.file_name(), timeout.as_secs()),
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        Ok(ToolOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
            duration: started.elapsed(),
        })
    }
}
