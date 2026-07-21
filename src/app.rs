use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use adw::prelude::*;
use gtk::gdk;
use gtk::glib::{self, ControlFlow};
use url::Url;

use crate::backend::{BackendEvent, BackendHandle, NodeMode};
use crate::config::{ConnectionMode, StoredConfig, config_path, load_or_create, save};
use crate::discovery::DiscoveredPeer;
use crate::system_proxy;

const APP_ID: &str = "io.github.ergolyam.Drosophila";
const PROTOCOLS: [&str; 5] = ["tcp", "tls", "ws", "wss", "quic"];

pub fn run(arguments: &[String]) -> glib::ExitCode {
    configure_windows_runtime();
    if let Err(error) = system_proxy::recover_stale() {
        tracing::warn!(%error, "failed to recover stale system proxy settings");
    }
    let application = adw::Application::builder().application_id(APP_ID).build();
    let (backend, events) = BackendHandle::spawn();
    let events = Rc::new(RefCell::new(Some(events)));

    application.connect_activate({
        let backend = backend.clone();
        let events = events.clone();
        move |application| {
            if let Some(window) = application.active_window() {
                window.present();
                return;
            }
            let Some(events) = events.borrow_mut().take() else {
                return;
            };
            match Ui::new(application, backend.clone(), events) {
                Ok(ui) => ui.window.present(),
                Err(error) => {
                    tracing::error!(%error, "failed to initialize UI");
                    application.quit();
                }
            }
        }
    });
    application.connect_shutdown(move |_| backend.shutdown());
    application.run_with_args(arguments)
}

#[cfg(not(windows))]
fn configure_windows_runtime() {}

#[cfg(windows)]
#[allow(unsafe_code)]
fn configure_windows_runtime() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = executable.parent() else {
        return;
    };
    let share = directory.join("share");
    let loaders = directory.join("lib/gdk-pixbuf-2.0/2.10.0/loaders");
    let loader_cache = directory.join("lib/gdk-pixbuf-2.0/2.10.0/loaders.cache");
    let schemas = share.join("glib-2.0/schemas");

    // SAFETY: this runs before the Tokio backend or any other application
    // threads are created, so no thread can read the process environment while
    // these variables are being changed.
    unsafe {
        std::env::set_var("XDG_DATA_DIRS", &share);
        std::env::set_var("GSETTINGS_SCHEMA_DIR", schemas);
        std::env::set_var("GDK_PIXBUF_MODULEDIR", loaders);
        std::env::set_var("GDK_PIXBUF_MODULE_FILE", loader_cache);
    }
    if let Err(error) = std::env::set_current_dir(directory) {
        tracing::warn!(%error, "failed to select the portable application directory");
    }
}

struct Ui {
    window: adw::ApplicationWindow,
    toast_overlay: adw::ToastOverlay,
    ygg_card: adw::ExpanderRow,
    address_row: adw::ActionRow,
    subnet_row: adw::ActionRow,
    private_key_row: adw::EntryRow,
    private_key_regen: gtk::Button,
    connection_group: adw::PreferencesGroup,
    mode_row: adw::ComboRow,
    proxy_card: adw::ExpanderRow,
    proxy_listen_row: adw::EntryRow,
    proxy_dns_ip_row: adw::EntryRow,
    proxy_dns_port_row: adw::EntryRow,
    peers_box: gtk::ListBox,
    peers_group: adw::PreferencesGroup,
    add_peer_button: gtk::MenuButton,
    config_path: PathBuf,
    config: RefCell<StoredConfig>,
    backend: BackendHandle,
    events: RefCell<Receiver<BackendEvent>>,
    running: Cell<bool>,
    transitioning: Cell<bool>,
    suppress_ygg_switch: Cell<bool>,
    suppress_mode_change: Cell<bool>,
    peer_icons: RefCell<HashMap<String, (gtk::Image, String)>>,
    peer_delete_buttons: RefCell<Vec<gtk::Button>>,
    discovery: RefCell<Option<Rc<DiscoveryDialog>>>,
    discovery_cache: RefCell<DiscoveryCache>,
    next_discovery_id: Cell<u64>,
}

impl Ui {
    fn new(
        application: &adw::Application,
        backend: BackendHandle,
        events: Receiver<BackendEvent>,
    ) -> anyhow::Result<Rc<Self>> {
        let config_path = config_path()?;
        let config = load_or_create(&config_path)?;

        let builder = gtk::Builder::new();
        builder.add_from_string(include_str!("../resources/ui.ui"))?;
        builder.add_from_string(include_str!("../resources/main.ui"))?;
        builder.add_from_string(include_str!("../resources/settings.ui"))?;

        let window: adw::ApplicationWindow = object(&builder, "main_window")?;
        window.set_application(Some(application));
        let stack: adw::ViewStack = object(&builder, "stack")?;
        let main: gtk::Box = object(&builder, "main")?;
        let settings: gtk::Box = object(&builder, "settings")?;
        let private_key_row: adw::EntryRow = object(&builder, "private_key_row")?;
        let main_page = stack.add_titled(&main, Some("main"), "Main");
        main_page.set_icon_name(Some("go-home-symbolic"));
        let settings_page = stack.add_titled(&settings, Some("settings"), "Settings");
        settings_page.set_icon_name(Some("emblem-system-symbolic"));
        stack.set_visible_child(&main);
        main.set_focusable(true);
        settings.set_focusable(true);
        stack.connect_visible_child_notify({
            let main = main.clone();
            let settings = settings.clone();
            let private_key_row = private_key_row.clone();
            move |stack| {
                if stack.visible_child_name().as_deref() == Some("settings") {
                    settings.grab_focus();
                    private_key_row.set_position(-1);
                } else {
                    main.grab_focus();
                }
            }
        });

        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("../resources/ui.css"));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let ui = Rc::new(Self {
            window,
            toast_overlay: object(&builder, "toast_overlay")?,
            ygg_card: object(&builder, "ygg_card")?,
            address_row: object(&builder, "address_row")?,
            subnet_row: object(&builder, "subnet_row")?,
            private_key_row,
            private_key_regen: object(&builder, "private_key_regen_icon")?,
            connection_group: object(&builder, "connection_group")?,
            mode_row: object(&builder, "mode_row")?,
            proxy_card: object(&builder, "proxy_card")?,
            proxy_listen_row: object(&builder, "proxy_listen_row")?,
            proxy_dns_ip_row: object(&builder, "proxy_dns_ip_row")?,
            proxy_dns_port_row: object(&builder, "proxy_dns_port_row")?,
            peers_box: object(&builder, "peers_box")?,
            peers_group: object(&builder, "peers_group")?,
            add_peer_button: object(&builder, "add_peer_btn")?,
            config_path,
            config: RefCell::new(config),
            backend,
            events: RefCell::new(events),
            running: Cell::new(false),
            transitioning: Cell::new(false),
            suppress_ygg_switch: Cell::new(false),
            suppress_mode_change: Cell::new(false),
            peer_icons: RefCell::new(HashMap::new()),
            peer_delete_buttons: RefCell::new(Vec::new()),
            discovery: RefCell::new(None),
            discovery_cache: RefCell::new(DiscoveryCache::default()),
            next_discovery_id: Cell::new(1),
        });

        ui.initialize_widgets();
        ui.connect_signals(&builder);
        ui.start_event_pump();
        Ok(ui)
    }

    fn initialize_widgets(self: &Rc<Self>) {
        self.ygg_card.set_title("Yggdrasil-ng");
        self.set_ygg_switch(false);
        self.address_row.set_subtitle("–");
        self.subnet_row.set_subtitle("–");
        self.private_key_row
            .set_text(&self.config.borrow().yggdrasil.private_key);

        let gui = self.config.borrow().drosophila.clone();
        #[cfg(feature = "tun")]
        self.mode_row.set_model(Some(&gtk::StringList::new(&[
            "TUN",
            "Proxy",
            "System Proxy",
        ])));
        #[cfg(not(feature = "tun"))]
        self.mode_row
            .set_model(Some(&gtk::StringList::new(&["Proxy", "System Proxy"])));
        self.connection_group.set_description(Some(if cfg!(feature = "tun") {
            "System Proxy updates desktop settings. Proxy only exposes a local endpoint. TUN routes applications that do not use proxy settings."
        } else {
            "System Proxy updates desktop settings. Proxy only exposes a local endpoint."
        }));
        self.set_connection_mode(gui.effective_mode());
        self.proxy_listen_row.set_text(&gui.proxy_listen);
        self.proxy_dns_ip_row.set_text(&gui.dns_server);
        self.proxy_dns_port_row.set_text(&gui.dns_port.to_string());
        self.update_proxy_card();
        self.rebuild_peers();
        self.suppress_ygg_switch.set(false);
        self.suppress_mode_change.set(false);
    }

    fn connect_signals(self: &Rc<Self>, builder: &gtk::Builder) {
        self.ygg_card.connect_enable_expansion_notify({
            let weak = Rc::downgrade(self);
            move |row| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_ygg_switch(row.enables_expansion());
                }
            }
        });
        self.mode_row.connect_selected_notify({
            let weak = Rc::downgrade(self);
            move |row| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_connection_mode_changed(row.selected());
                }
            }
        });

        self.connect_copy_row(&self.address_row);
        self.connect_copy_row(&self.subnet_row);

        self.private_key_row.connect_changed({
            let weak = Rc::downgrade(self);
            move |row| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_private_key_changed(&row.text());
                }
            }
        });
        self.private_key_regen.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.regenerate_private_key();
                }
            }
        });
        self.proxy_listen_row.connect_changed({
            let weak = Rc::downgrade(self);
            move |row| {
                if let Some(ui) = weak.upgrade() {
                    row.text()
                        .trim()
                        .clone_into(&mut ui.config.borrow_mut().drosophila.proxy_listen);
                    ui.update_proxy_card();
                    ui.persist_config();
                }
            }
        });
        self.proxy_dns_ip_row.connect_changed({
            let weak = Rc::downgrade(self);
            move |row| {
                if let Some(ui) = weak.upgrade() {
                    row.text()
                        .trim()
                        .clone_into(&mut ui.config.borrow_mut().drosophila.dns_server);
                    ui.persist_config();
                }
            }
        });
        self.proxy_dns_port_row.connect_changed({
            let weak = Rc::downgrade(self);
            move |row| {
                let Some(ui) = weak.upgrade() else { return };
                match row.text().trim().parse::<u16>() {
                    Ok(port) if port != 0 => {
                        row.remove_css_class("error");
                        ui.config.borrow_mut().drosophila.dns_port = port;
                        ui.persist_config();
                    }
                    _ => row.add_css_class("error"),
                }
            }
        });

        let add_manual: gtk::Button = object(builder, "add_peer_manual_btn").unwrap();
        add_manual.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.add_peer_button.popdown();
                    ui.open_add_peer_dialog(None);
                }
            }
        });
        let add_find: gtk::Button = object(builder, "add_peer_find_btn").unwrap();
        add_find.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.add_peer_button.popdown();
                    ui.open_discovery();
                }
            }
        });

        let about: gtk::Button = object(builder, "about_btn").unwrap();
        about.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_about();
                }
            }
        });
    }

    fn connect_copy_row(self: &Rc<Self>, row: &adw::ActionRow) {
        let gesture = gtk::GestureClick::new();
        gesture.connect_released({
            let weak = Rc::downgrade(self);
            let row = row.clone();
            move |_, _, _, _| {
                let text = row.subtitle().unwrap_or_default();
                if text.is_empty() || text == "–" {
                    return;
                }
                if let Some(ui) = weak.upgrade() {
                    ui.window.clipboard().set_text(&text);
                    ui.toast("Copied to clipboard");
                }
            }
        });
        row.add_controller(gesture);
    }

    fn on_ygg_switch(self: &Rc<Self>, desired: bool) {
        if self.suppress_ygg_switch.replace(false) {
            return;
        }
        if self.transitioning.get() {
            self.set_ygg_switch(self.running.get());
            return;
        }
        if desired == self.running.get() {
            return;
        }
        self.transitioning.set(true);
        self.set_controls_sensitive(false);
        if desired {
            self.ygg_card.set_subtitle("Starting…");
            self.backend.start(self.config.borrow().clone());
        } else {
            self.ygg_card.set_subtitle("Stopping…");
            self.backend.stop();
        }
    }

    fn on_connection_mode_changed(&self, selected: u32) {
        if self.suppress_mode_change.replace(false) {
            return;
        }
        #[cfg(feature = "tun")]
        let mode = match selected {
            0 => ConnectionMode::Tun,
            1 => ConnectionMode::Proxy,
            _ => ConnectionMode::SystemProxy,
        };
        #[cfg(not(feature = "tun"))]
        let mode = if selected == 0 {
            ConnectionMode::Proxy
        } else {
            ConnectionMode::SystemProxy
        };
        self.config.borrow_mut().drosophila.mode = mode;
        self.update_proxy_card();
        self.persist_config();
        if self.running.get() {
            self.toast("Connection mode will be applied after restarting Yggdrasil");
        }
    }

    fn on_private_key_changed(&self, text: &str) {
        let key = text.trim();
        let valid = key.len() == 128 && key.bytes().all(|byte| byte.is_ascii_hexdigit());
        if valid {
            self.private_key_row.remove_css_class("error");
            if self.config.borrow().yggdrasil.private_key != key {
                key.clone_into(&mut self.config.borrow_mut().yggdrasil.private_key);
                self.persist_config();
            }
        } else {
            self.private_key_row.add_css_class("error");
        }
    }

    fn regenerate_private_key(&self) {
        self.config.borrow_mut().regenerate_private_key();
        let key = self.config.borrow().yggdrasil.private_key.clone();
        self.private_key_row.set_text(&key);
        self.persist_config();
        if self.running.get() {
            self.toast("The new key will be applied after restarting Yggdrasil");
        }
    }

    fn start_event_pump(self: &Rc<Self>) {
        let ui = self.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            while let Ok(event) = ui.events.borrow().try_recv() {
                ui.handle_backend_event(event);
            }
            ControlFlow::Continue
        });
    }

    fn handle_backend_event(self: &Rc<Self>, event: BackendEvent) {
        match event {
            BackendEvent::Starting => {
                self.transitioning.set(true);
                self.set_controls_sensitive(false);
                self.ygg_card.set_subtitle("Starting…");
            }
            BackendEvent::Started {
                address,
                subnet,
                mode,
            } => {
                self.running.set(true);
                self.transitioning.set(false);
                self.set_ygg_switch(true);
                self.ygg_card.set_expanded(true);
                self.ygg_card.set_subtitle(match mode {
                    NodeMode::Tun => "Running",
                    NodeMode::SystemProxy => "System proxy running",
                    NodeMode::Proxy => "Proxy running",
                });
                self.address_row.set_subtitle(&address);
                self.subnet_row.set_subtitle(&subnet);
                self.set_controls_sensitive(true);
            }
            BackendEvent::Stopped => self.show_stopped(),
            BackendEvent::Failed(message) => {
                if self.transitioning.get() {
                    self.show_stopped();
                }
                self.show_error("Yggdrasil-ng Error", &message);
            }
            BackendEvent::PeerStatus(status) => self.apply_peer_status(&status),
            BackendEvent::DiscoveryFinished { id, result } => {
                if let Some(dialog) = self.discovery.borrow().as_ref()
                    && dialog.id.get() == id
                {
                    dialog.set_result(result);
                }
            }
        }
    }

    fn show_stopped(&self) {
        self.running.set(false);
        self.transitioning.set(false);
        self.set_ygg_switch(false);
        self.ygg_card.set_expanded(false);
        self.ygg_card.set_subtitle("Stopped");
        self.address_row.set_subtitle("–");
        self.subnet_row.set_subtitle("–");
        self.set_controls_sensitive(true);
        self.apply_peer_status(&HashMap::new());
    }

    fn set_ygg_switch(&self, enabled: bool) {
        if self.ygg_card.enables_expansion() != enabled {
            self.suppress_ygg_switch.set(true);
            self.ygg_card.set_enable_expansion(enabled);
        }
    }

    fn set_connection_mode(&self, mode: ConnectionMode) {
        #[cfg(feature = "tun")]
        let selected = match mode {
            ConnectionMode::Tun => 0,
            ConnectionMode::Proxy => 1,
            ConnectionMode::SystemProxy => 2,
        };
        #[cfg(not(feature = "tun"))]
        let selected = match mode {
            ConnectionMode::Proxy => 0,
            ConnectionMode::SystemProxy | ConnectionMode::Tun => 1,
        };
        if self.mode_row.selected() != selected {
            self.suppress_mode_change.set(true);
            self.mode_row.set_selected(selected);
        }
    }

    fn update_proxy_card(&self) {
        let config = self.config.borrow();
        let mode = config.drosophila.effective_mode();
        self.proxy_card.set_visible(mode != ConnectionMode::Tun);
        self.proxy_card.set_title(match mode {
            ConnectionMode::SystemProxy => "System Proxy Details",
            ConnectionMode::Proxy | ConnectionMode::Tun => "Proxy Details",
        });
        let listen = config.drosophila.proxy_listen.clone();
        self.proxy_card
            .set_subtitle(&format!("HTTP and SOCKS5 on {listen}"));
    }

    fn set_controls_sensitive(&self, sensitive: bool) {
        self.ygg_card.set_sensitive(sensitive);
        self.private_key_regen.set_sensitive(sensitive);
        let connection_sensitive = sensitive && !self.running.get();
        self.mode_row.set_sensitive(connection_sensitive);
        self.proxy_card.set_sensitive(connection_sensitive);
        for button in self.peer_delete_buttons.borrow().iter() {
            button.set_sensitive(sensitive);
        }
    }

    fn persist_config(&self) {
        if let Err(error) = save(&self.config_path, &self.config.borrow()) {
            tracing::error!(%error, "failed to save configuration");
            self.toast("Failed to save configuration");
        }
    }

    fn toast(&self, message: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(message));
    }

    fn show_error(&self, heading: &str, body: &str) {
        let dialog = adw::AlertDialog::builder()
            .heading(heading)
            .body(body)
            .close_response("ok")
            .build();
        dialog.add_response("ok", "OK");
        dialog.present(Some(&self.window));
    }

    fn rebuild_peers(self: &Rc<Self>) {
        while let Some(child) = self.peers_box.first_child() {
            self.peers_box.remove(&child);
        }
        self.peer_icons.borrow_mut().clear();
        self.peer_delete_buttons.borrow_mut().clear();

        let mut peers = self.config.borrow().yggdrasil.peers.clone();
        peers.sort();
        for peer in peers {
            let (title, subtitle, icon_name) = peer_display(&peer);
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .activatable(false)
                .build();
            row.add_css_class("compact");
            let icon = gtk::Image::from_icon_name(icon_name);
            row.add_prefix(&icon);
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .css_classes(["destructive-action", "flat"])
                .build();
            row.add_suffix(&remove);
            remove.connect_clicked({
                let weak = Rc::downgrade(self);
                let peer = peer.clone();
                move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.remove_peer(&peer);
                    }
                }
            });
            self.peers_box.append(&row);
            self.peer_delete_buttons.borrow_mut().push(remove);
            self.peer_icons.borrow_mut().insert(
                without_query(&peer).to_owned(),
                (icon, icon_name.to_owned()),
            );
        }

        let count = self.config.borrow().yggdrasil.peers.len();
        if count == 0 {
            self.peers_group
                .set_description(Some("No peers configured"));
        } else {
            self.peers_group.set_description(Some(&format!(
                "{count} peer node{}",
                if count == 1 { "" } else { "s" }
            )));
        }
    }

    fn remove_peer(self: &Rc<Self>, peer: &str) {
        self.config
            .borrow_mut()
            .yggdrasil
            .peers
            .retain(|configured| configured != peer);
        self.persist_config();
        self.rebuild_peers();
        if self.running.get() {
            self.backend.remove_peer(peer.to_owned());
        }
    }

    fn add_peer(self: &Rc<Self>, peer: String) {
        if self.config.borrow().yggdrasil.peers.contains(&peer) {
            return;
        }
        self.config.borrow_mut().yggdrasil.peers.push(peer.clone());
        self.persist_config();
        self.rebuild_peers();
        if self.running.get() {
            self.backend.add_peer(peer);
        }
    }

    fn apply_peer_status(&self, status: &HashMap<String, bool>) {
        for (peer, (icon, default_icon)) in self.peer_icons.borrow().iter() {
            let icon_name = if status.get(peer).copied().unwrap_or(false) {
                default_icon
            } else if self.running.get() {
                "network-error-symbolic"
            } else {
                default_icon
            };
            icon.set_icon_name(Some(icon_name));
        }
    }

    fn open_add_peer_dialog(self: &Rc<Self>, prefill: Option<DiscoveredPeer>) {
        let builder = gtk::Builder::from_string(include_str!("../resources/peer_dialog.ui"));
        let dialog: adw::AlertDialog = object(&builder, "add_peer_dialog").unwrap();
        let domain: adw::EntryRow = object(&builder, "domain_row").unwrap();
        let protocol: adw::ComboRow = object(&builder, "proto_row").unwrap();
        let sni: adw::EntryRow = object(&builder, "sni_row").unwrap();
        let password: adw::PasswordEntryRow = object(&builder, "password_row").unwrap();
        let sni_group: adw::PreferencesGroup = object(&builder, "sni_group").unwrap();
        let domain_error: gtk::Label = object(&builder, "domain_error").unwrap();
        let sni_error: gtk::Label = object(&builder, "sni_error").unwrap();

        if let Some(peer) = prefill {
            let selected = PROTOCOLS
                .iter()
                .position(|value| *value == peer.protocol)
                .unwrap_or_default();
            protocol.set_selected(u32::try_from(selected).unwrap_or_default());
            domain.set_text(&format_endpoint(&peer.host, peer.port));
            if let Ok(url) = Url::parse(&peer.address) {
                for (key, value) in url.query_pairs() {
                    match key.as_ref() {
                        "sni" => sni.set_text(&value),
                        "password" => password.set_text(&value),
                        _ => {}
                    }
                }
            }
        }

        let validate: Rc<dyn Fn()> = Rc::new({
            let dialog = dialog.clone();
            let domain = domain.clone();
            let protocol = protocol.clone();
            let sni = sni.clone();
            let sni_group = sni_group.clone();
            let domain_error = domain_error.clone();
            let sni_error = sni_error.clone();
            move || {
                let proto = PROTOCOLS[protocol.selected() as usize];
                let endpoint_ok = validate_endpoint(proto, domain.text().trim());
                domain_error.set_visible(!endpoint_ok && !domain.text().is_empty());
                if endpoint_ok {
                    domain.remove_css_class("error");
                } else if !domain.text().is_empty() {
                    domain.add_css_class("error");
                }
                let tls = proto == "tls";
                sni_group.set_visible(tls);
                let sni_ok =
                    !tls || sni.text().trim().is_empty() || valid_hostname(sni.text().trim());
                sni_error.set_visible(!sni_ok);
                if sni_ok {
                    sni.remove_css_class("error");
                } else {
                    sni.add_css_class("error");
                }
                dialog.set_response_enabled("add", endpoint_ok && sni_ok);
            }
        });
        domain.connect_changed({
            let validate = validate.clone();
            move |_| validate()
        });
        sni.connect_changed({
            let validate = validate.clone();
            move |_| validate()
        });
        protocol.connect_selected_notify({
            let validate = validate.clone();
            move |_| validate()
        });
        validate();

        dialog.connect_response(None, {
            let weak = Rc::downgrade(self);
            move |_, response| {
                if response != "add" {
                    return;
                }
                let Some(ui) = weak.upgrade() else { return };
                let proto = PROTOCOLS[protocol.selected() as usize];
                if let Some(peer) = build_peer_uri(
                    proto,
                    domain.text().trim(),
                    sni.text().trim(),
                    password.text().trim(),
                ) {
                    ui.add_peer(peer);
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    fn open_discovery(self: &Rc<Self>) {
        let dialog = DiscoveryDialog::new(Rc::downgrade(self));
        *self.discovery.borrow_mut() = Some(dialog.clone());
        dialog.start(false);
        dialog.dialog.present(Some(&self.window));
    }

    fn open_about(&self) {
        let builder = gtk::Builder::from_string(include_str!("../resources/about_dialog.ui"));
        let about: adw::AboutDialog = object(&builder, "about_dialog").unwrap();
        about.set_version(env!("CARGO_PKG_VERSION"));
        about.present(Some(&self.window));
    }
}

struct DiscoveryDialog {
    app: Weak<Ui>,
    dialog: adw::AlertDialog,
    list: gtk::ListBox,
    sort: adw::ComboRow,
    spinner: gtk::Spinner,
    progress_box: gtk::Box,
    progress_label: gtk::Label,
    filters: Vec<(String, gtk::CheckButton)>,
    peers: RefCell<Vec<DiscoveredPeer>>,
    visible: RefCell<Vec<DiscoveredPeer>>,
    id: Cell<u64>,
    request_protocols: RefCell<Vec<String>>,
}

impl DiscoveryDialog {
    fn new(app: Weak<Ui>) -> Rc<Self> {
        let builder = gtk::Builder::from_string(include_str!("../resources/peer_discovery.ui"));
        let filters = PROTOCOLS
            .iter()
            .map(|protocol| {
                (
                    (*protocol).to_owned(),
                    object(&builder, &format!("filter_{protocol}")).unwrap(),
                )
            })
            .collect();
        let this = Rc::new(Self {
            app,
            dialog: object(&builder, "peer_discovery_dialog").unwrap(),
            list: object(&builder, "peer_discovery_list").unwrap(),
            sort: object(&builder, "peer_sort_row").unwrap(),
            spinner: object(&builder, "peer_discovery_spinner").unwrap(),
            progress_box: object(&builder, "peer_discovery_progress_box").unwrap(),
            progress_label: object(&builder, "peer_discovery_progress_label").unwrap(),
            filters,
            peers: RefCell::new(Vec::new()),
            visible: RefCell::new(Vec::new()),
            id: Cell::new(0),
            request_protocols: RefCell::new(Vec::new()),
        });
        this.connect(&builder);
        this
    }

    fn connect(self: &Rc<Self>, builder: &gtk::Builder) {
        self.dialog.set_response_enabled("use", false);
        for (protocol, button) in &self.filters {
            button.set_active(protocol == "tcp" || protocol == "tls");
            button.connect_toggled({
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(dialog) = weak.upgrade() {
                        dialog.refresh_list();
                    }
                }
            });
        }
        self.sort.connect_selected_notify({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(dialog) = weak.upgrade() {
                    dialog.refresh_list();
                }
            }
        });
        self.list.connect_row_selected({
            let dialog = self.dialog.clone();
            move |_, row| dialog.set_response_enabled("use", row.is_some())
        });
        let refresh: gtk::Button = object(builder, "refresh_peers_btn").unwrap();
        refresh.connect_clicked({
            let weak = Rc::downgrade(self);
            move |_| {
                if let Some(dialog) = weak.upgrade() {
                    dialog.start(true);
                }
            }
        });
        self.dialog.connect_response(None, {
            let weak = Rc::downgrade(self);
            move |_, response| {
                let Some(dialog) = weak.upgrade() else { return };
                let Some(app) = dialog.app.upgrade() else {
                    return;
                };
                if response == "use"
                    && let Some(row) = dialog.list.selected_row()
                    && let Ok(index) = usize::try_from(row.index())
                    && let Some(peer) = dialog.visible.borrow().get(index).cloned()
                {
                    app.open_add_peer_dialog(Some(peer));
                }
                *app.discovery.borrow_mut() = None;
            }
        });
    }

    fn selected_protocols(&self) -> Vec<String> {
        self.filters
            .iter()
            .filter(|(_, button)| button.is_active())
            .map(|(protocol, _)| protocol.clone())
            .collect()
    }

    fn start(&self, refresh: bool) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let protocols = self.selected_protocols();
        if protocols.is_empty() {
            self.progress_label
                .set_label("Select at least one protocol");
            return;
        }
        (*self.request_protocols.borrow_mut()).clone_from(&protocols);
        if !refresh && let Some(peers) = app.discovery_cache.borrow().get(&protocols) {
            *self.peers.borrow_mut() = peers.to_vec();
            self.refresh_list();
            self.progress_box.set_visible(true);
            self.spinner.stop();
            self.spinner.set_visible(false);
            return;
        }
        let id = app.next_discovery_id.get();
        app.next_discovery_id.set(id.wrapping_add(1));
        self.id.set(id);
        self.peers.borrow_mut().clear();
        self.visible.borrow_mut().clear();
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.progress_box.set_visible(true);
        self.spinner.set_visible(true);
        self.spinner.start();
        self.progress_label.set_label("Searching for peers…");
        self.dialog.set_response_enabled("use", false);
        app.backend.discover(id, protocols);
    }

    fn set_result(&self, result: Result<Vec<DiscoveredPeer>, String>) {
        self.spinner.stop();
        self.spinner.set_visible(false);
        match result {
            Ok(peers) => {
                if let Some(app) = self.app.upgrade() {
                    app.discovery_cache
                        .borrow_mut()
                        .insert(&self.request_protocols.borrow(), peers.clone());
                }
                *self.peers.borrow_mut() = peers;
                self.refresh_list();
            }
            Err(error) => {
                tracing::warn!(%error, "peer discovery failed");
                self.progress_label
                    .set_label("Search failed. See log for details.");
                if let Some(app) = self.app.upgrade() {
                    app.discovery_cache
                        .borrow_mut()
                        .remove(&self.request_protocols.borrow());
                }
            }
        }
    }

    fn refresh_list(&self) {
        let enabled: Vec<&str> = self
            .filters
            .iter()
            .filter(|(_, button)| button.is_active())
            .map(|(protocol, _)| protocol.as_str())
            .collect();
        let mut visible: Vec<_> = self
            .peers
            .borrow()
            .iter()
            .filter(|peer| enabled.contains(&peer.protocol.as_str()))
            .cloned()
            .collect();
        if self.sort.selected() == 1 {
            visible.sort_by_key(|peer| (peer.country.to_lowercase(), peer.ping_ms));
        } else {
            visible.sort_by_key(|peer| (peer.ping_ms.unwrap_or(u64::MAX), peer.country.clone()));
        }
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for peer in &visible {
            let row = adw::ActionRow::builder()
                .title(format_endpoint(&peer.host, peer.port))
                .subtitle(format!(
                    "{} • {}",
                    peer.protocol.to_uppercase(),
                    peer.country
                ))
                .activatable(true)
                .build();
            let ping = gtk::Label::new(Some(
                &peer
                    .ping_ms
                    .map_or_else(|| "–".to_owned(), |ping| format!("{ping} ms")),
            ));
            ping.set_xalign(1.0);
            row.add_suffix(&ping);
            self.list.append(&row);
        }
        let status = if visible.is_empty() {
            "No peers available".to_owned()
        } else {
            format!("Shown {} peers", visible.len())
        };
        self.progress_label.set_label(&status);
        *self.visible.borrow_mut() = visible;
    }
}

fn object<T: glib::object::IsA<glib::Object> + Clone + 'static>(
    builder: &gtk::Builder,
    id: &str,
) -> anyhow::Result<T> {
    builder
        .object(id)
        .ok_or_else(|| anyhow::anyhow!("UI object {id} is missing or has the wrong type"))
}

fn validate_endpoint(protocol: &str, endpoint: &str) -> bool {
    let Ok(url) = Url::parse(&format!("{protocol}://{endpoint}")) else {
        return false;
    };
    url.host().is_some() && url.port_or_known_default().is_some()
}

fn valid_hostname(value: &str) -> bool {
    static HOSTNAME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    HOSTNAME
        .get_or_init(|| {
            regex::Regex::new(
                r"^(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)+[A-Za-z]{2,63}$",
            )
            .unwrap()
        })
        .is_match(value)
}

fn build_peer_uri(protocol: &str, endpoint: &str, sni: &str, password: &str) -> Option<String> {
    let mut url = Url::parse(&format!("{protocol}://{endpoint}")).ok()?;
    {
        let mut query = url.query_pairs_mut();
        if protocol == "tls" && !sni.is_empty() {
            query.append_pair("sni", sni);
        }
        if !password.is_empty() {
            query.append_pair("password", password);
        }
    }
    Some(url.to_string().trim_end_matches('/').to_owned())
}

fn peer_display(peer: &str) -> (String, String, &'static str) {
    let Ok(url) = Url::parse(peer) else {
        return (
            peer.to_owned(),
            "Invalid peer URI".to_owned(),
            "network-error-symbolic",
        );
    };
    let host = url.host_str().unwrap_or("Unknown");
    let title = url
        .port_or_known_default()
        .map_or_else(|| host.to_owned(), |port| format_endpoint(host, port));
    let mut parts = vec![url.scheme().to_uppercase()];
    for (key, value) in url.query_pairs() {
        if key == "sni" {
            parts.push(format!("SNI: {value}"));
        } else if key == "password" {
            parts.push("Password".to_owned());
        }
    }
    let icon = match url.scheme() {
        "quic" => "network-transmit-receive-symbolic",
        "ws" | "wss" => "web-browser-symbolic",
        _ => "network-wired-symbolic",
    };
    (title, parts.join(" • "), icon)
}

fn format_endpoint(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn without_query(uri: &str) -> &str {
    uri.split_once('?').map_or(uri, |(base, _)| base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_builder_percent_encodes_secrets() {
        let peer = build_peer_uri("tls", "example.com:443", "front.example", "a b").unwrap();
        assert!(peer.contains("sni=front.example"));
        assert!(peer.contains("password=a+b"));
    }

    #[test]
    fn ipv6_endpoint_is_bracketed() {
        assert_eq!(format_endpoint("2001:db8::1", 443), "[2001:db8::1]:443");
    }
}
#[derive(Default)]
struct DiscoveryCache {
    entries: HashMap<Vec<String>, Vec<DiscoveredPeer>>,
}

impl DiscoveryCache {
    fn key(protocols: &[String]) -> Vec<String> {
        let mut protocols = protocols.to_vec();
        protocols.sort_unstable();
        protocols
    }

    fn get(&self, protocols: &[String]) -> Option<&[DiscoveredPeer]> {
        self.entries.get(&Self::key(protocols)).map(Vec::as_slice)
    }

    fn insert(&mut self, protocols: &[String], peers: Vec<DiscoveredPeer>) {
        self.entries.insert(Self::key(protocols), peers);
    }

    fn remove(&mut self, protocols: &[String]) {
        self.entries.remove(&Self::key(protocols));
    }
}

#[cfg(test)]
mod discovery_cache_tests {
    use super::DiscoveryCache;

    fn protocols(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn protocol_order_does_not_affect_cache_lookup() {
        let mut cache = DiscoveryCache::default();
        cache.insert(&protocols(&["tcp", "tls"]), Vec::new());

        assert!(cache.get(&protocols(&["tls", "tcp"])).is_some());
    }

    #[test]
    fn removing_request_keeps_other_cached_protocols() {
        let mut cache = DiscoveryCache::default();
        cache.insert(&protocols(&["tcp", "tls"]), Vec::new());
        cache.insert(&protocols(&["ws"]), Vec::new());

        cache.remove(&protocols(&["tls", "tcp"]));

        assert!(cache.get(&protocols(&["tcp", "tls"])).is_none());
        assert!(cache.get(&protocols(&["ws"])).is_some());
    }
}
