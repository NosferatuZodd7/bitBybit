//! Windows integration test: the fixed tool runner starts System32 tools only, with validated arguments.
//! Runs `pnputil /enum-drivers` (read-only) and checks that hostile arguments never start anything.
#![cfg(windows)]

use bitbybit_platform_windows::tools::{SystemToolRunner, resolve};
use bitbybit_security::tools::{FixedTool, ToolRunner};
use std::time::Duration;

#[test]
fn runs_pnputil_read_only_and_rejects_hostile_arguments() {
    let path = resolve(FixedTool::PnpUtil).expect("pnputil in System32");
    assert!(path.to_string_lossy().to_ascii_lowercase().ends_with(r"system32\pnputil.exe"), "{}", path.display());
    let out = SystemToolRunner.run(FixedTool::PnpUtil, &["/enum-drivers".into()], Duration::from_secs(120)).unwrap();
    assert!(out.succeeded(), "exit {} {}", out.exit_code, out.stderr);
    assert!(!out.stdout.is_empty());
    for bad in ["/enum-drivers\" & calc", "a\nb"] {
        assert!(SystemToolRunner.run(FixedTool::PnpUtil, &[bad.into()], Duration::from_secs(5)).is_err());
    }
}
