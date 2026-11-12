//! Windows integration test: the real update check against this repository's GitHub releases over WinHTTP
//! (download, ECDSA verification with the embedded key, validation). Read-only; installs nothing.
#![cfg(windows)]

use bitbybit_updates::SemVersion;
use bitbybit_updates::check::check;

#[test]
fn a_published_release_is_found_downloaded_and_verified() {
    let repository = "NosferatuZodd7/bitBybit";
    let fetch = |url: &str| bitbybit_platform_windows::http::get(url, 8 * 1024 * 1024);
    let offer = check(&fetch, repository, "testing", SemVersion::parse("0.0.1").as_ref())
        .expect("check")
        .expect("a signed release is offered");
    println!("offered {} ({} bytes, sha256 {})", offer.release.tag, offer.app.size, offer.app.sha256);
    assert!(offer.app.size > 1_000_000);
    assert!(
        check(&fetch, repository, "testing", SemVersion::parse("999.0.0").as_ref()).expect("check").is_none(),
        "nothing newer than 999"
    );
}
