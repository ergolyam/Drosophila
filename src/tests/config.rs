use super::*;

#[test]
fn round_trip_preserves_yggdrasil_and_gui_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("yggdrasil.toml");
    let mut expected = StoredConfig::default();
    expected.yggdrasil.peers = vec!["tls://example.com:443".to_owned()];
    expected.drosophila.mode = ConnectionMode::SystemProxy;

    save(&path, &expected).unwrap();
    let actual = load_or_create(&path).unwrap();

    assert_eq!(actual.yggdrasil.peers, expected.yggdrasil.peers);
    assert_eq!(actual.drosophila.mode, ConnectionMode::SystemProxy);
    assert_eq!(actual.yggdrasil.admin_listen, "none");
}

#[test]
fn legacy_proxy_switch_migrates_to_connection_mode() {
    let proxy: GuiConfig = toml::from_str("proxy_enabled = true").unwrap();
    let tun: GuiConfig = toml::from_str("proxy_enabled = false").unwrap();

    assert_eq!(proxy.mode, ConnectionMode::Proxy);
    assert_eq!(tun.mode, ConnectionMode::Tun);
}

#[cfg(not(feature = "tun"))]
#[test]
fn userspace_proxy_remains_available_without_tun() {
    let mut config = GuiConfig {
        mode: ConnectionMode::Proxy,
        ..GuiConfig::default()
    };
    assert_eq!(config.effective_mode(), ConnectionMode::Proxy);

    config.mode = ConnectionMode::Tun;
    assert_eq!(config.effective_mode(), ConnectionMode::SystemProxy);
}
