//! End-to-end smoke against a running broker (`bitbybit-service.exe --console`) on the real pipe. Opt-in:
//! runs only with BITBYBIT_SERVICE_SMOKE=1 (the Windows CI job starts the broker and enables developer mode for
//! its target directory). Read-only operations only.
#![cfg(windows)]

use bitbybit_ipc::builtin::{Empty, PingResponse};
use bitbybit_ipc::client::BrokerClient;
use bitbybit_ipc::protocol::PIPE_NAME;
use bitbybit_platform_windows::pipe::connect;
use std::time::Duration;

#[test]
fn ping_and_network_overview_through_the_running_broker() {
    if std::env::var("BITBYBIT_SERVICE_SMOKE").as_deref() != Ok("1") {
        eprintln!("skipped: set BITBYBIT_SERVICE_SMOKE=1 with a running broker");
        return;
    }
    let stream = connect(PIPE_NAME, Duration::from_secs(20)).expect("broker pipe");
    let client = BrokerClient::new(stream.clone(), stream);
    let ping: PingResponse = client.invoke("broker.ping", &Empty {}, None, Duration::from_secs(20)).expect("ping");
    assert!(ping.developer_mode, "CI enables developer mode for the test binaries");
    assert!(ping.operations.contains(&"network.overview".to_string()));
    let overview: serde_json::Value =
        client.invoke("network.overview", &Empty {}, None, Duration::from_secs(30)).expect("overview");
    let interfaces = overview["snapshot"]["interfaces"].as_array().map_or(0, Vec::len);
    assert!(interfaces > 0, "the broker read the runner's interfaces");
    println!("broker {} – {interfaces} interfaces, connectivity {}", ping.broker_version, overview["connectivity"]);
}
