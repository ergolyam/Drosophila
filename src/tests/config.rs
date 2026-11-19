use super::*;

#[test]
fn round_trip_preserves_yggdrasil_and_gui_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("yggdrasil.toml");
    let mut expected = StoredConfig::default();
    expected.yggdrasil.peers = vec!["tls://example.com:443".to_owned()];
    expected.drosophila.mode = ConnectionMode::Proxy;
    expected.drosophila.tray_enabled = true;

    save(&path, &expected).unwrap();
    let actual = load_or_create(&path).unwrap();

    assert_eq!(actual.yggdrasil.peers, expected.yggdrasil.peers);
    assert_eq!(actual.drosophila.mode, ConnectionMode::Proxy);
    assert!(actual.drosophila.tray_enabled);
    assert_eq!(actual.yggdrasil.admin_listen, "none");
}

#[test]
fn legacy_proxy_switch_migrates_to_connection_mode() {
    let proxy: GuiConfig = toml::from_str("proxy_enabled = true").unwrap();
    let tun: GuiConfig = toml::from_str("proxy_enabled = false").unwrap();

    assert_eq!(proxy.mode, ConnectionMode::Proxy);
    assert_eq!(tun.mode, ConnectionMode::Tun);
    assert!(!proxy.tray_enabled);
    assert!(!tun.tray_enabled);
}

#[test]
fn connection_mode_defaults_to_tun() {
    assert_eq!(GuiConfig::default().mode, ConnectionMode::Tun);
}

#[test]
fn tray_enabled_defaults_to_false() {
    assert!(!GuiConfig::default().tray_enabled);
    let config: GuiConfig = toml::from_str("").unwrap();
    assert!(!config.tray_enabled);
}

#[test]
fn tray_enabled_round_trips_both_values() {
    for enabled in [false, true] {
        let config = GuiConfig {
            tray_enabled: enabled,
            ..GuiConfig::default()
        };
        let text = toml::to_string(&config).unwrap();
        let actual: GuiConfig = toml::from_str(&text).unwrap();
        assert_eq!(actual.tray_enabled, enabled);
    }
}

#[test]
fn missing_gui_section_defaults_tray_to_false() {
    let mut value = toml::Value::try_from(StoredConfig::default()).unwrap();
    value.as_table_mut().unwrap().remove("drosophila");
    let config: StoredConfig = value.try_into().unwrap();
    assert!(!config.drosophila.tray_enabled);
}
