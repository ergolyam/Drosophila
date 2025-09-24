import json, time
from threading import Thread

from gi.repository import Gtk, Adw, GLib # type: ignore
from urllib.parse import urlparse, parse_qs

from yggui.exec.get_info import get_peers_status

from yggui.core.common import Runtime, Gui, Regexp


def apply_status(app, status: dict[str, bool]) -> None:
    for peer, (icon, default_icon) in getattr(app, "_peer_icons", {}).items():
        icon.set_from_icon_name(default_icon if status.get(peer, False) else 'network-error-symbolic')


def update_peer_status(app) -> bool:
    if getattr(app, "ygg_pid", None) is None and getattr(app, "socks_pid", None) is None:
        clear_peer_status(app)
        return False

    use_socks = getattr(app, "socks_config", {}).get("enabled", False)
    status = get_peers_status(use_socks)
    GLib.idle_add(apply_status, app, status)

    if status or getattr(app, "_peer_status_thread_running", False):
        return False

    def _poll_until_found() -> None:
        deadline = time.time() + 15
        while (
            time.time() < deadline
            and not getattr(app, "_stop_peer_status_thread", False)
        ):
            st = get_peers_status(use_socks)
            if st:
                GLib.idle_add(apply_status, app, st)
                break
            time.sleep(1)

        GLib.idle_add(setattr, app, "_peer_status_thread_running", False)

    app._stop_peer_status_thread = False
    app._peer_status_thread_running = True
    Thread(target=_poll_until_found, daemon=True).start()
    return False


def clear_peer_status(app) -> bool:
    for icon, default_icon in getattr(app, "_peer_icons", {}).values():
        icon.set_from_icon_name(default_icon)

    app._stop_peer_status_thread = True
    app._peer_status_thread_running = False
    return False


def read_config():
    if Runtime.config_path.exists():
        try:
            with open(Runtime.config_path, "r", encoding="utf-8") as handle:
                return json.load(handle)
        except Exception:
            return {}
    return {}


def write_config(cfg):
    with open(Runtime.config_path, "w", encoding="utf-8") as handle:
        json.dump(cfg, handle, indent=2)


def save_peers_to_disk(app):
    cfg = read_config()
    cfg["Peers"] = app.peers
    write_config(cfg)


def build_proto_widget(proto: str) -> tuple[Gtk.Widget, str, Gtk.Image]:
    base_name = {
        "tcp": "network-wired-symbolic",
        "tls": "network-wired-symbolic",
        "quic": "network-transmit-receive-symbolic",
        "ws": "web-browser-symbolic",
        "wss": "web-browser-symbolic",
    }.get(proto, "network-server-symbolic")

    base_img = Gtk.Image.new_from_icon_name(base_name)

    if proto in {"tls", "wss"}:
        overlay = Gtk.Overlay()
        overlay.set_child(base_img)
        lock = Gtk.Image.new_from_icon_name("security-high-symbolic")
        lock.set_pixel_size(12)
        lock.set_valign(Gtk.Align.END)
        lock.set_margin_bottom(2)
        overlay.add_overlay(lock)
        return overlay, base_name, base_img

    return base_img, base_name, base_img


def rebuild_peers_box(app):
    child = app.peers_box.get_first_child()
    while child:
        nxt = child.get_next_sibling()
        app.peers_box.remove(child)
        child = nxt

    app._peer_rows = {}
    app._peer_icons = {}

    for peer in sorted(app.peers):
        parsed = urlparse(peer)
        proto = parsed.scheme
        host = parsed.hostname or parsed.netloc.split("?", 1)[0]
        port = f":{parsed.port}" if parsed.port else ""
        title = f"{host}{port}"

        row = Adw.ActionRow()
        row.set_title(title)
        row.add_css_class("compact")

        subtitle_parts = [proto.upper()]
        if proto == "tls":
            sni = parse_qs(parsed.query).get("sni", [None])[0]
            if sni:
                subtitle_parts.append(f"SNI: {sni}")
        row.set_subtitle(" • ".join(subtitle_parts))
        row.set_activatable(False)

        icon_widget, icon_name, image_widget = build_proto_widget(proto)
        row.add_prefix(icon_widget)

        trash_btn = Gtk.Button()
        trash_btn.set_icon_name("user-trash-symbolic")
        trash_btn.add_css_class("destructive-action")
        trash_btn.add_css_class("flat")
        row.add_suffix(trash_btn)

        trash_btn.connect("clicked", lambda _b, p=peer: remove_peer(app, p))
        app.peers_box.append(row)

        peer_key = peer.split("?", 1)[0]
        app._peer_rows[peer_key] = row
        app._peer_icons[peer_key] = (image_widget, icon_name)

    count = len(app.peers)
    if count == 0:
        app.peers_group.set_description("No peers configured")
    else:
        plural = "s" if count != 1 else ""
        app.peers_group.set_description(f"{count} peer node{plural}")


def open_add_peer_dialog(app):
    builder = Gtk.Builder.new_from_file(str(Gui.peer_ui_file))

    dialog: Adw.AlertDialog = builder.get_object("add_peer_dialog")
    domain_row: Adw.EntryRow = builder.get_object("domain_row")
    proto_row: Adw.ComboRow = builder.get_object("proto_row")
    sni_row: Adw.EntryRow = builder.get_object("sni_row")
    sni_group = builder.get_object("sni_group")
    domain_error: Gtk.Label = builder.get_object("domain_error")
    sni_error:    Gtk.Label = builder.get_object("sni_error")

    def _update_sni_row(_row=None, _pspec=None):
        tls_selected = proto_row.get_selected() == 1
        sni_group.set_visible(tls_selected)
        if not tls_selected:
            sni_error.add_css_class("hidden-error")
        _validate()

    proto_row.connect("notify::selected", _update_sni_row)

    def _validate(_row=None, _pspec=None):
        domain = domain_row.get_text().strip()
        sni    = sni_row.get_text().strip()
        proto  = ["tcp", "tls", "quic", "ws", "wss"][proto_row.get_selected()]

        domain_has_text = bool(domain)
        domain_valid    = bool(Regexp.domain_re.fullmatch(domain))

        if domain_has_text and not domain_valid:
            domain_row.add_css_class("error")
            domain_error.remove_css_class("hidden-error")
        else:
            domain_row.remove_css_class("error")
            domain_error.add_css_class("hidden-error")

        sni_valid = True
        if proto == "tls":
            sni_has_text = bool(sni)
            sni_valid = (not sni_has_text) or bool(Regexp.sni_re.fullmatch(sni))

            if sni_has_text and not sni_valid:
                sni_row.add_css_class("error")
                sni_error.remove_css_class("hidden-error")
            else:
                sni_row.remove_css_class("error")
                sni_error.add_css_class("hidden-error")
        else:
            sni_row.remove_css_class("error")
            sni_error.add_css_class("hidden-error")

        dialog.set_response_enabled("add", domain_valid and sni_valid)

    domain_row.connect("notify::text", _validate)
    sni_row.connect("notify::text",    _validate)

    _update_sni_row()

    def _commit():
        domain = domain_row.get_text().strip()
        if not domain:
            return

        proto = ["tcp", "tls", "quic", "ws", "wss"][proto_row.get_selected()]
        peer  = f"{proto}://{domain}"

        sni = sni_row.get_text().strip()
        if proto == "tls" and sni:
            peer += f"?sni={sni}"

        if peer not in app.peers:
            app.peers.append(peer)
            save_peers_to_disk(app)
            rebuild_peers_box(app)
            if app.ygg_pid is not None or app.socks_pid is not None:
                from yggui.funcs.ygg import request_ygg_state
                request_ygg_state(app, False)
                request_ygg_state(app, True)
            update_peer_status(app)

    dialog.connect("response", lambda _d, resp: resp == "add" and _commit())
    dialog.present(app.win)


def load_config(app):
    cfg = read_config()
    app.peers = cfg.get("Peers", [])
    rebuild_peers_box(app)
    if not getattr(app, "_add_btn_connected", False):
        app.add_peer_btn.connect("clicked", lambda _b: open_add_peer_dialog(app))
        app._add_btn_connected = True


def remove_peer(app, peer):
    if peer in app.peers:
        app.peers.remove(peer)
        save_peers_to_disk(app)
        rebuild_peers_box(app)
        if app.ygg_pid is not None or app.socks_pid is not None:
            from yggui.funcs.ygg import request_ygg_state
            request_ygg_state(app, False)
            request_ygg_state(app, True)
        update_peer_status(app)


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")

