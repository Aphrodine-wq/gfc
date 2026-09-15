use gfc_config::Config;

#[test]
fn example_config_parses() {
    let text = include_str!("../../../config.example.toml");
    let cfg: Config = toml::from_str(text).expect("example config");
    cfg.validate().expect("valid");
    assert!(!cfg.plugins.summarizer_enabled);
    assert!(!cfg.webhook.enabled);
}
