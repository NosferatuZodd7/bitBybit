//! End-to-end tests: a real `BrokerClient` talking to `connection::serve` over OS pipes, with test-only
//! operations (no system state is touched).

use crate::auth::AuthorizationPolicy;
use crate::auth::tests::{APP, client};
use crate::builtin::{Empty, Ping, PingResponse, PollEvents};
use crate::client::BrokerClient;
use crate::confirm::TokenService;
use crate::dispatch::{Dispatcher, DispatcherOptions};
use crate::envelope::{BrokerRequest, ResponseStatus};
use crate::events::EventHub;
use crate::identity::ClientIdentity;
use crate::operation::{BrokerOperation, CallContext, Category, Descriptor, RegistryBuilder, RiskTier};
use crate::plan::OperationPlan;
use bitbybit_core::{ErrorCode, Result, RiskLevel, text};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Text {
    text: String,
}

struct Echo(Descriptor);
impl BrokerOperation for Echo {
    type Request = Text;
    type Response = Text;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &Text) -> Result<()> {
        if r.text.len() > 100 { Err(bitbybit_core::Error::invalid("too long")) } else { Ok(()) }
    }
    fn execute(&self, r: &Text, _: &CallContext) -> Result<Text> {
        Ok(r.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Hotspot {
    ssid: String,
    passphrase: String,
}

/// Medium-risk mutation; counts executions.
struct ConfigureHotspot(Descriptor, Arc<AtomicUsize>);
impl BrokerOperation for ConfigureHotspot {
    type Request = Hotspot;
    type Response = Empty;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, _: &Hotspot) -> Result<()> {
        Ok(())
    }
    fn plan(&self, r: &Hotspot, _: &CallContext) -> Result<Option<OperationPlan>> {
        let mut plan = OperationPlan::new(
            text!("test.plan", "Configure {0}", r.ssid.clone()),
            vec![text!("test.step", "Restart the hotspot")],
            RiskLevel::Medium,
        );
        plan.may_interrupt_connectivity = true;
        Ok(Some(plan))
    }
    fn execute(&self, _: &Hotspot, ctx: &CallContext) -> Result<Empty> {
        assert!(ctx.is_confirmed);
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(Empty {})
    }
    fn redact_for_audit(&self, r: &Hotspot) -> Hotspot {
        Hotspot { passphrase: "***".into(), ..r.clone() }
    }
}

/// Low risk, no plan; panics or sleeps on request.
struct Touch(Descriptor);
impl BrokerOperation for Touch {
    type Request = Text;
    type Response = Empty;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, _: &Text) -> Result<()> {
        Ok(())
    }
    fn execute(&self, r: &Text, _: &CallContext) -> Result<Empty> {
        match r.text.as_str() {
            "panic" => panic!("secret internal detail"),
            "slow" => std::thread::sleep(Duration::from_millis(400)),
            _ => {}
        }
        Ok(Empty {})
    }
}

fn d(name: &'static str, tier: RiskTier, category: Category) -> Descriptor {
    Descriptor { name, tier, category, requires_confirmation: false, timeout: None }
}

struct Broker {
    dispatcher: Arc<Dispatcher>,
    executed: Arc<AtomicUsize>,
}

fn broker() -> Broker {
    let executed = Arc::new(AtomicUsize::new(0));
    let ping = Ping::new("0.7.0-test", false);
    let names = ping.operations.clone();
    let registry = RegistryBuilder::default()
        .register(ping)
        .and_then(|r| r.register(PollEvents::new(Arc::new(EventHub::new(16)))))
        .and_then(|r| r.register(Echo(d("test.echo", RiskTier::ReadOnly, Category::NONE))))
        .and_then(|r| {
            r.register(ConfigureHotspot(
                d("network.configureHotspot", RiskTier::Standard, Category::NETWORK),
                executed.clone(),
            ))
        })
        .and_then(|r| r.register(Touch(d("network.touch", RiskTier::Standard, Category::NETWORK))))
        .and_then(|r| r.register(Touch(d("drivers.touch", RiskTier::Elevated, Category::DRIVERS))))
        .unwrap()
        .freeze();
    names.set(registry.names()).unwrap();
    let policy = AuthorizationPolicy::new(&[APP], None).unwrap();
    let options = DispatcherOptions { busy_wait: Duration::from_millis(50), ..DispatcherOptions::default() };
    Broker { dispatcher: Arc::new(Dispatcher::new(registry, policy, TokenService::new().unwrap(), options)), executed }
}

fn connect(b: &Broker, identity: ClientIdentity) -> BrokerClient<std::io::PipeWriter> {
    let (server_in, client_out) = std::io::pipe().unwrap();
    let (client_in, server_out) = std::io::pipe().unwrap();
    let dispatcher = b.dispatcher.clone();
    std::thread::spawn(move || crate::connection::serve(&dispatcher, server_in, server_out, move || identity));
    BrokerClient::new(client_in, client_out)
}

fn call(
    b: &Broker,
    client_identity: ClientIdentity,
    op: &str,
    parameters: serde_json::Value,
) -> crate::envelope::BrokerResponse {
    let request = BrokerRequest {
        protocol_version: 1,
        request_id: Uuid::new_v4(),
        operation: op.into(),
        parameters,
        confirmation_token: None,
    };
    b.dispatcher.dispatch(&request, &client_identity, Arc::new(AtomicBool::new(false)))
}

#[test]
fn ping_and_echo_round_trip() {
    let b = broker();
    let c = connect(&b, client(APP, false));
    let ping: PingResponse = c.invoke("broker.ping", &Empty {}, None, WAIT).unwrap();
    assert_eq!((ping.protocol_version, ping.operations.len()), (1, 6));
    assert!(ping.operations.contains(&"test.echo".to_string()));
    let echo: Text = c.invoke("test.echo", &Text { text: "olá".into() }, None, WAIT).unwrap();
    assert_eq!(echo.text, "olá");
}

#[test]
fn unknown_operation_invalid_parameters_and_unauthorized_client_fail_safely() {
    let b = broker();
    let me = client(APP, false);
    let r = call(&b, me.clone(), "test.nothing", serde_json::json!({}));
    assert_eq!(r.error.map(|e| e.code), Some(ErrorCode::NotFound));
    let r = call(&b, me.clone(), "test.echo", serde_json::json!({"text": "a", "extra": true}));
    assert_eq!(r.error.as_ref().map(|e| e.code), Some(ErrorCode::InvalidArgument));
    assert_eq!(r.error.and_then(|e| e.user_text).map(|t| t.key).as_deref(), Some("ipc.error.malformedRequest"));
    let r = call(&b, me, "test.echo", serde_json::json!({"text": "x".repeat(101)}));
    assert_eq!(r.error.map(|e| e.code), Some(ErrorCode::InvalidArgument));
    let r = call(&b, client(r"C:\Users\x\evil.exe", true), "test.echo", serde_json::json!({"text": "a"}));
    assert_eq!(r.error.and_then(|e| e.user_text).map(|t| t.key).as_deref(), Some("ipc.error.clientNotAuthorized"));
}

#[test]
fn medium_risk_operation_runs_only_after_confirmation() {
    let b = broker();
    let c = connect(&b, client(APP, false));
    let request = Hotspot { ssid: "Casa".into(), passphrase: "hunter2hunter2".into() };
    let without = c.invoke::<_, Empty>("network.configureHotspot", &request, None, WAIT).unwrap_err();
    assert_eq!(without.code, ErrorCode::ConfirmationRequired);
    let declined = c.invoke::<_, Empty>("network.configureHotspot", &request, Some(&|_, _| false), WAIT).unwrap_err();
    assert_eq!(declined.code, ErrorCode::Cancelled);
    assert_eq!(b.executed.load(Ordering::SeqCst), 0);
    let seen = std::sync::Mutex::new(None);
    let confirm = |plan: &OperationPlan, expires: Option<time::OffsetDateTime>| {
        *seen.lock().unwrap() = Some((plan.operation.clone(), plan.may_interrupt_connectivity, expires.is_some()));
        true
    };
    c.invoke::<_, Empty>("network.configureHotspot", &request, Some(&confirm), WAIT).unwrap();
    assert_eq!(b.executed.load(Ordering::SeqCst), 1);
    assert_eq!(seen.lock().unwrap().clone(), Some(("network.configureHotspot".into(), true, true)));
}

#[test]
fn tokens_cannot_be_replayed_or_moved_to_other_parameters() {
    let b = broker();
    let me = client(APP, false);
    let params = serde_json::json!({"ssid": "Casa", "passphrase": "x"});
    let first = call(&b, me.clone(), "network.configureHotspot", params.clone());
    assert_eq!(first.status, ResponseStatus::ConfirmationRequired);
    let token = first.confirmation_token.unwrap();
    let send = |parameters: serde_json::Value| {
        let request = BrokerRequest {
            protocol_version: 1,
            request_id: Uuid::new_v4(),
            operation: "network.configureHotspot".into(),
            parameters,
            confirmation_token: Some(token.clone()),
        };
        b.dispatcher.dispatch(&request, &me, Arc::new(AtomicBool::new(false)))
    };
    assert_eq!(
        send(serde_json::json!({"ssid": "Other", "passphrase": "x"})).error.map(|e| e.code),
        Some(ErrorCode::ConfirmationRequired)
    );
    assert_eq!(send(params.clone()).status, ResponseStatus::Ok);
    assert_eq!(send(params).error.map(|e| e.code), Some(ErrorCode::ConfirmationRequired), "single use");
}

#[test]
fn elevated_operation_is_refused_for_unelevated_client() {
    let b = broker();
    let r = call(&b, client(APP, false), "drivers.touch", serde_json::json!({"text": "a"}));
    assert_eq!(r.error.as_ref().map(|e| e.code), Some(ErrorCode::AccessDenied));
    assert_eq!(r.error.and_then(|e| e.user_text).map(|t| t.key).as_deref(), Some("ipc.error.elevationRequired"));
    assert_eq!(
        call(&b, client(APP, true), "drivers.touch", serde_json::json!({"text": "a"})).status,
        ResponseStatus::Ok
    );
}

#[test]
fn low_risk_mutation_needs_no_confirmation_and_panics_are_not_leaked() {
    let b = broker();
    assert_eq!(
        call(&b, client(APP, false), "network.touch", serde_json::json!({"text": "ok"})).status,
        ResponseStatus::Ok
    );
    let r = call(&b, client(APP, false), "network.touch", serde_json::json!({"text": "panic"}));
    let e = r.error.unwrap();
    assert_eq!((e.code, e.user_text.map(|t| t.key).as_deref()), (ErrorCode::PlatformError, Some("ipc.error.internal")));
    assert!(!e.message.contains("secret"));
}

#[test]
fn concurrent_mutations_in_the_same_category_report_busy() {
    let b = broker();
    let c = connect(&b, client(APP, false));
    std::thread::scope(|s| {
        let slow = s.spawn(|| c.invoke::<_, Empty>("network.touch", &Text { text: "slow".into() }, None, WAIT));
        std::thread::sleep(Duration::from_millis(100));
        let busy = c.invoke::<_, Empty>("network.touch", &Text { text: "ok".into() }, None, WAIT).unwrap_err();
        assert_eq!(busy.code, ErrorCode::Busy);
        let other = c.invoke::<Text, Text>("test.echo", &Text { text: "read".into() }, None, WAIT);
        assert!(other.is_ok(), "read-only calls are not gated");
        assert!(slow.join().unwrap().is_ok());
    });
}

#[test]
fn malformed_frames_close_the_connection() {
    let b = broker();
    let (server_in, mut client_out) = std::io::pipe().unwrap();
    let (mut client_in, server_out) = std::io::pipe().unwrap();
    let dispatcher = b.dispatcher.clone();
    let server =
        std::thread::spawn(move || crate::connection::serve(&dispatcher, server_in, server_out, || client(APP, false)));
    crate::framing::write_frame(&mut client_out, br#"{"protocolVersion":1,"protocolVersion":1}"#, 1024).unwrap();
    let crate::framing::FrameRead::Frame(reply) = crate::framing::read_frame(&mut client_in, 1 << 20) else {
        panic!("expected an error reply")
    };
    assert!(String::from_utf8_lossy(&reply).contains("malformedRequest"));
    server.join().unwrap();
    assert_eq!(crate::framing::read_frame(&mut client_in, 1 << 20), crate::framing::FrameRead::EndOfStream);
}

#[test]
fn registry_rules_are_enforced() {
    let dup = RegistryBuilder::default()
        .register(Echo(d("test.echo", RiskTier::ReadOnly, Category::NONE)))
        .and_then(|r| r.register(Echo(d("test.Echo", RiskTier::ReadOnly, Category::NONE))));
    assert!(dup.is_err(), "duplicates differing only in case");
    assert!(
        RegistryBuilder::default().register(Echo(d("test.mutate", RiskTier::Standard, Category::NONE))).is_err(),
        "mutations need a category"
    );
    assert!(RegistryBuilder::default().register(Echo(d("Bad.name", RiskTier::ReadOnly, Category::NONE))).is_err());
    let long = Descriptor {
        timeout: Some(Duration::from_secs(31 * 60)),
        ..d("test.long", RiskTier::ReadOnly, Category::NONE)
    };
    assert!(RegistryBuilder::default().register(Echo(long)).is_err());
}

#[test]
fn all_ipc_texts_exist_in_catalog() {
    use crate::envelope::texts::*;
    let catalog = bitbybit_localization::TextCatalog::default_catalog();
    for t in [
        unsupported_protocol(2),
        malformed_request(),
        confirmation_invalid(),
        unknown_operation("a.b"),
        client_not_authorized(),
        elevation_required(),
        busy("network"),
        timeout(),
        internal(),
    ] {
        assert!(catalog.template(&t.key).is_some(), "{}", t.key);
    }
}
