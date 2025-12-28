import time
from threading import Thread

from gi.repository import GLib, Gtk  # type: ignore

from yggui.funcs.peers import (
    update_peer_status,
    clear_peer_status,
    set_trash_buttons_sensitive
)
from yggui.exec.toggle import (
    start_ygg,
    stop_ygg
)
from yggui.exec.get_info import get_self_info


def drain_pending(app) -> bool:
    desired = getattr(app, "pending_switch_state", None)
    app.pending_switch_state = None
    if desired is None:
        return False
    running = app.ygg_pid is not None or app.socks_pid is not None
    if desired != running:
        GLib.idle_add(app.ygg_card.set_enable_expansion, desired)
    return False


def request_ygg_state(app, desired: bool) -> None:
    app.pending_switch_state = desired
    if not getattr(app, "switch_locked", False):
        GLib.idle_add(app.ygg_card.set_enable_expansion, desired)


def set_switch_lock(app, locked: bool) -> None:
    app.switch_locked = locked
    try:
        app.ygg_card.set_sensitive(not locked)
        app.private_key_regen_icon.set_sensitive(not locked)
        set_trash_buttons_sensitive(app, not locked)
    except Exception:
        pass


def show_error_dialog(app, message: str) -> None:
    dialog = Gtk.MessageDialog(
        transient_for=app.win,
        modal=True,
        buttons=Gtk.ButtonsType.OK,
        message_type=Gtk.MessageType.ERROR,
        text="Error while running Yggdrasil",
        secondary_text=message,
    )
    dialog.connect("response", lambda d, _r: d.destroy())
    dialog.show()


def on_process_error(app, message: str) -> bool:
    show_error_dialog(app, message)

    if app.ygg_card.get_enable_expansion():
        app.ygg_card.set_enable_expansion(False)

    app.switch_row.set_subtitle("Stopped")
    app.ygg_pid = None
    app._set_ip_labels("-", "-")
    app._expand_ipv6_card(False)
    set_switch_lock(app, False)
    drain_pending(app)
    return False


def poll_for_addresses(app, use_socks) -> None:
    deadline = time.time() + 15
    while time.time() < deadline and (
        app.ygg_pid is not None or app.socks_pid is not None
    ):
        addr, subnet = get_self_info(use_socks)
        if addr and subnet:
            GLib.idle_add(app._set_ip_labels, addr, subnet)
            GLib.idle_add(update_peer_status, app)
            GLib.idle_add(set_switch_lock, app, False)
            GLib.idle_add(drain_pending, app)
            return
        time.sleep(1)

    GLib.idle_add(app._set_ip_labels, "-", "-")
    GLib.idle_add(update_peer_status, app)
    GLib.idle_add(set_switch_lock, app, False)
    GLib.idle_add(drain_pending, app)


def switch_switched(app, _switch, state: bool) -> None:
    use_socks = getattr(app, "socks_config", {}).get("enabled", False)
    if getattr(app, "switch_locked", False):
        app.pending_switch_state = state
        running = app.ygg_pid is not None or app.socks_pid is not None
        if state is not running:
            app.ygg_card.set_enable_expansion(running)
        return

    if state and app.ygg_pid is None and app.socks_pid is None:
        set_switch_lock(app, True)
        try:
            pid = start_ygg(use_socks, app.socks_config)
            if use_socks:
                app.socks_pid = pid
            else:
                app.ygg_pid = pid
        except Exception as exc:
            GLib.idle_add(
                on_process_error,
                app,
                f"Failed to start {'Yggstack' if use_socks else 'Yggdrasil'}: {exc}",
            )
            return

        app.switch_row.set_subtitle("Running")
        app._set_ip_labels("-", "-")
        app._expand_ipv6_card(True)

        Thread(target=poll_for_addresses, args=(app, use_socks), daemon=True).start()

    elif not state and (app.ygg_pid is not None or app.socks_pid is not None):
        set_switch_lock(app, True)
        pid = app.ygg_pid or app.socks_pid
        use_socks = pid is app.socks_pid
        app.ygg_pid = app.socks_pid = None
        if pid:
            stop_ygg(use_socks, pid)
        app.switch_row.set_subtitle("Stopped")
        app._set_ip_labels("-", "-")
        app._expand_ipv6_card(False)
        clear_peer_status(app)
        set_switch_lock(app, False)
        drain_pending(app)

    print(f"The switch has been switched {'on' if state else 'off'}")


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")

