use std::sync::mpsc::Sender;

#[derive(Debug)]
pub enum TrayEvent {
    Show(Option<String>),
    Toggle,
    Quit,
    #[cfg(target_os = "linux")]
    Offline(u64),
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Sender, TrayEvent};
    use gtk::gdk_pixbuf::Pixbuf;
    use gtk::gio;
    use ksni::blocking::TrayMethods;

    struct TrayItem {
        sender: Sender<TrayEvent>,
        icon: ksni::Icon,
        running: bool,
        transitioning: bool,
        generation: u64,
    }

    impl TrayItem {
        fn show(&self) {
            let mut token = ksni::current_xdg_activation_token();
            if token.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(1));
                token = ksni::current_xdg_activation_token();
            }
            let _ = self.sender.send(TrayEvent::Show(token));
        }
    }

    impl ksni::Tray for TrayItem {
        fn id(&self) -> String {
            "io.github.ergolyam.Drosophila".into()
        }

        fn title(&self) -> String {
            "Drosophila".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![self.icon.clone()]
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.show();
        }

        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            self.show();
        }

        fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
            let _ = self.sender.send(TrayEvent::Offline(self.generation));
            false
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};

            vec![
                StandardItem {
                    label: "Open".into(),
                    activate: Box::new(|tray: &mut Self| tray.show()),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: if self.running { "Stop" } else { "Start" }.into(),
                    enabled: !self.transitioning,
                    activate: Box::new(|tray: &mut Self| {
                        if !tray.transitioning {
                            let _ = tray.sender.send(TrayEvent::Toggle);
                        }
                    }),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Quit".into(),
                    activate: Box::new(|tray: &mut Self| {
                        let _ = tray.sender.send(TrayEvent::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub struct Tray {
        handle: ksni::blocking::Handle<TrayItem>,
    }

    impl Tray {
        pub fn new(sender: Sender<TrayEvent>, generation: u64) -> anyhow::Result<Self> {
            let bytes = gtk::glib::Bytes::from_static(include_bytes!(
                "../xdg/io.github.ergolyam.Drosophila-tray.svg"
            ));
            let stream = gio::MemoryInputStream::from_bytes(&bytes);
            let pixbuf =
                Pixbuf::from_stream_at_scale(&stream, 64, 64, true, None::<&gio::Cancellable>)?;
            let pixels = pixbuf.read_pixel_bytes();
            let channels = usize::try_from(pixbuf.n_channels())?;
            let stride = usize::try_from(pixbuf.rowstride())?;
            let width = usize::try_from(pixbuf.width())?;
            let height = usize::try_from(pixbuf.height())?;
            let mut data = Vec::with_capacity(width * height * 4);
            for row in 0..height {
                for pixel in pixels[row * stride..][..width * channels].chunks_exact(channels) {
                    data.extend_from_slice(&[
                        if channels == 4 { pixel[3] } else { 255 },
                        pixel[0],
                        pixel[1],
                        pixel[2],
                    ]);
                }
            }
            let icon = ksni::Icon {
                width: pixbuf.width(),
                height: pixbuf.height(),
                data,
            };
            let handle = TrayItem {
                sender,
                icon,
                running: false,
                transitioning: false,
                generation,
            }
            .disable_dbus_name(crate::config::is_flatpak())
            .spawn()?;
            Ok(Self { handle })
        }

        pub fn set_state(&self, running: bool, transitioning: bool) {
            self.handle.update(|tray| {
                tray.running = running;
                tray.transitioning = transitioning;
            });
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            drop(self.handle.shutdown());
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{Sender, TrayEvent};
    use gtk::glib::{self, ControlFlow};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    static ACTIVE: AtomicBool = AtomicBool::new(false);

    pub struct Tray {
        _icon: TrayIcon,
        toggle: MenuItem,
        source: Option<glib::SourceId>,
    }

    impl Tray {
        pub fn new(sender: Sender<TrayEvent>, _generation: u64) -> anyhow::Result<Self> {
            anyhow::ensure!(!ACTIVE.swap(true, Ordering::AcqRel), "tray already exists");
            let result = Self::create(sender);
            if result.is_err() {
                ACTIVE.store(false, Ordering::Release);
            }
            result
        }

        fn create(sender: Sender<TrayEvent>) -> anyhow::Result<Self> {
            let menu = Menu::new();
            let open = MenuItem::new("Open", true, None);
            let toggle = MenuItem::new("Start", true, None);
            let quit = MenuItem::new("Quit", true, None);
            menu.append_items(&[&open, &toggle, &PredefinedMenuItem::separator(), &quit])?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_icon(Icon::from_resource(2, None)?)
                .with_tooltip("Drosophila")
                .with_menu_on_left_click(true)
                .with_menu_on_right_click(false)
                .build()?;
            let icon_id = icon.id().clone();
            let open_id = open.id().clone();
            let toggle_id = toggle.id().clone();
            let quit_id = quit.id().clone();
            let source =
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                        if let TrayIconEvent::Click {
                            id,
                            button: MouseButton::Right,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            if id == icon_id {
                                let _ = sender.send(TrayEvent::Show(None));
                            }
                        }
                    }
                    while let Ok(event) = MenuEvent::receiver().try_recv() {
                        let action = if event.id == open_id {
                            Some(TrayEvent::Show(None))
                        } else if event.id == toggle_id {
                            Some(TrayEvent::Toggle)
                        } else if event.id == quit_id {
                            Some(TrayEvent::Quit)
                        } else {
                            None
                        };
                        if let Some(action) = action {
                            let _ = sender.send(action);
                        }
                    }
                    ControlFlow::Continue
                });
            Ok(Self {
                _icon: icon,
                toggle,
                source: Some(source),
            })
        }

        pub fn set_state(&self, running: bool, transitioning: bool) {
            self.toggle.set_text(if running { "Stop" } else { "Start" });
            self.toggle.set_enabled(!transitioning);
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            if let Some(source) = self.source.take() {
                source.remove();
            }
            ACTIVE.store(false, Ordering::Release);
        }
    }
}

#[cfg(any(target_os = "linux", windows))]
pub use platform::Tray;
