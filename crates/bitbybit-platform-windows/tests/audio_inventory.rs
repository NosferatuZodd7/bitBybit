//! Windows integration test: reads the audio endpoints and defaults of the session it runs in. Read-only.
//! CI runners usually have no audio hardware (and may have the audio service stopped): an empty list or a
//! reported platform error is acceptable there; a real PC lists its devices.
#![cfg(windows)]

use bitbybit_audio::changes::AudioDevices;
use bitbybit_platform_windows::audio::CoreAudio;

#[test]
fn reads_endpoints_and_defaults_without_changing_anything() {
    match CoreAudio.endpoints() {
        Ok(endpoints) => {
            let defaults = CoreAudio.defaults().expect("defaults");
            assert_eq!(defaults.len(), 6, "2 flows x 3 roles");
            for e in &endpoints {
                assert!(!e.id.is_empty());
                println!(
                    "{:?} {:?} {:?} {:?} roles={:?}",
                    e.flow,
                    e.state,
                    e.display_name(),
                    e.form_factor,
                    e.default_roles
                );
            }
            println!("{} endpoint(s)", endpoints.len());
        }
        Err(e) => println!("no audio stack on this machine: {e}"),
    }
}
