use gfc_config::{PluginCapabilities, PluginEntry};
use gfc_plugin::{CapabilityRequest, PluginHost, Policy, HEALTH_CHECK_WORLD, SUMMARIZER_WORLD};
use std::path::PathBuf;

#[test]
fn capability_denial_is_host_enforced() {
    let entry = PluginEntry {
        path: PathBuf::from("sample.wasm"),
        world: HEALTH_CHECK_WORLD.into(),
        capabilities: PluginCapabilities::default(),
    };
    let policy = Policy::from_entry(&entry);
    let err = policy
        .authorize(&CapabilityRequest {
            network: vec!["example.com".into()],
            filesystem: vec![],
            process: false,
            credentials: vec!["github".into()],
        })
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("network") || msg.contains("credentials"));
}

#[test]
fn summarizer_off_by_default() {
    let host = PluginHost::new(&gfc_config::PluginsConfig::default()).unwrap();
    assert!(host.assert_world_allowed(SUMMARIZER_WORLD).is_err());
}
