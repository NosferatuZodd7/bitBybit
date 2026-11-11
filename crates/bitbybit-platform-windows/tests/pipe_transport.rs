//! Windows integration test: a real named pipe (unique test name) between two threads of this process, carrying
//! the broker protocol end to end. Touches no system state beyond the temporary pipe.
#![cfg(windows)]

use bitbybit_ipc::auth::AuthorizationPolicy;
use bitbybit_ipc::builtin::{Empty, Ping, PingResponse};
use bitbybit_ipc::client::BrokerClient;
use bitbybit_ipc::confirm::TokenService;
use bitbybit_ipc::dispatch::{Dispatcher, DispatcherOptions};
use bitbybit_ipc::identity::IntegrityLevel;
use bitbybit_ipc::operation::RegistryBuilder;
use bitbybit_platform_windows::pipe::{PipeServer, client_identity, connect, server_is_local_system};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn broker_protocol_over_a_real_named_pipe() {
    let name = format!("bitBybit.test.{}", std::process::id());
    let me = std::env::current_exe().unwrap();
    let ping = Ping::new("test", false);
    let names = ping.operations.clone();
    let registry = RegistryBuilder::default().register(ping).unwrap().freeze();
    names.set(registry.names()).unwrap();
    let policy = AuthorizationPolicy::new(&[me.to_str().unwrap()], None).unwrap();
    let dispatcher =
        Arc::new(Dispatcher::new(registry, policy, TokenService::new().unwrap(), DispatcherOptions::default()));

    let server = PipeServer::new(&name).unwrap();
    // Not joined: the client's reader thread holds its handle copy until its own read timeout.
    std::thread::spawn({
        let dispatcher = dispatcher.clone();
        move || {
            let stream = server.accept().unwrap();
            let identity = client_identity(&stream);
            let reader = stream.clone();
            bitbybit_ipc::connection::serve(&dispatcher, reader, stream, move || identity.clone());
        }
    });
    std::thread::sleep(Duration::from_millis(200));
    let stream = connect(&name, Duration::from_secs(5)).unwrap();
    assert!(!server_is_local_system(&stream), "the test server runs as the test user, not SYSTEM");
    let client = BrokerClient::new(stream.clone(), stream);
    let ping: PingResponse = client.invoke("broker.ping", &Empty {}, None, Duration::from_secs(10)).unwrap();
    assert_eq!(ping.operations, vec!["broker.ping".to_string()]);
}

#[test]
fn identity_of_this_process_is_resolved_from_the_pipe() {
    let name = format!("bitBybit.test.id.{}", std::process::id());
    let server = PipeServer::new(&name).unwrap();
    let accept = std::thread::spawn(move || {
        let s = server.accept().unwrap();
        client_identity(&s)
    });
    std::thread::sleep(Duration::from_millis(200));
    let _client = connect(&name, Duration::from_secs(5)).unwrap();
    let id = accept.join().unwrap();
    assert!(id.is_resolved(), "{id}");
    assert_eq!(id.process_id, std::process::id());
    assert!(id.integrity >= IntegrityLevel::Medium);
    let image = id.image_path.unwrap().to_lowercase();
    assert_eq!(image, std::env::current_exe().unwrap().to_string_lossy().to_lowercase());
    println!("user={:?} elevated={} session={:?}", id.user_sid, id.is_elevated, id.session_id);
}
