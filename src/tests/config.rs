use super::*;

#[test]
fn round_trip_preserves_yggdrasil_and_gui_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("yggdrasil.toml");
    let mut expected = StoredConfig::default();
    expected.yggdrasil.peers = vec!["tls://example.com:443".to_owned()];
    expected.drosophila.mode = ConnectionMode::Proxy;
    expected.drosophila.tray_enabled = true;
    expected.drosophila.background_enabled = true;

    save(&path, &expected).unwrap();
    let actual = load_or_create(&path).unwrap();

    assert_eq!(actual.yggdrasil.peers, expected.yggdrasil.peers);
    assert_eq!(actual.drosophila.mode, ConnectionMode::Proxy);
    assert!(actual.drosophila.tray_enabled);
    assert!(actual.drosophila.background_enabled);
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
fn independent_settings_default_to_false() {
    let defaults = GuiConfig::default();
    assert!(!defaults.tray_enabled);
    assert!(!defaults.background_enabled);
    let config: GuiConfig = toml::from_str("").unwrap();
    assert!(!config.tray_enabled);
    assert!(!config.background_enabled);
}

#[test]
fn independent_settings_round_trip_all_combinations() {
    for tray_enabled in [false, true] {
        for background_enabled in [false, true] {
            let config = GuiConfig {
                tray_enabled,
                background_enabled,
                ..GuiConfig::default()
            };
            let text = toml::to_string(&config).unwrap();
            let actual: GuiConfig = toml::from_str(&text).unwrap();
            assert_eq!(actual.tray_enabled, tray_enabled);
            assert_eq!(actual.background_enabled, background_enabled);
        }
    }
}

#[test]
fn missing_independent_setting_defaults_to_false() {
    for enabled in [false, true] {
        let config: GuiConfig = toml::from_str(&format!("tray_enabled = {enabled}")).unwrap();
        assert_eq!(config.tray_enabled, enabled);
        assert!(!config.background_enabled);

        let config: GuiConfig = toml::from_str(&format!("background_enabled = {enabled}")).unwrap();
        assert!(!config.tray_enabled);
        assert_eq!(config.background_enabled, enabled);
    }
}

#[test]
fn missing_gui_section_defaults_independent_settings_to_false() {
    let mut value = toml::Value::try_from(StoredConfig::default()).unwrap();
    value.as_table_mut().unwrap().remove("drosophila");
    let config: StoredConfig = value.try_into().unwrap();
    assert!(!config.drosophila.tray_enabled);
    assert!(!config.drosophila.background_enabled);
}
