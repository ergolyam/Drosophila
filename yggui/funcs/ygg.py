import time
from threading import Thread

from gi.repository import GLib, Gtk, Adw  # type: ignore

from yggui.funcs.peers import (
    update_peer_status,
    clear_peer_status,
    set_trash_buttons_sensitive
)
from yggui.exec.toggle import (
    start_ygg,
    stop_ygg
)
from yggui.exec.shell import Shell
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
    dialog = Adw.AlertDialog(
        heading="Yggdrasil Error",
        body=message,
        close_response="ok"
    )
    dialog.add_response("ok", "OK")
    dialog.present(app.win)


def on_process_error(app, message: str) -> bool:
    app.ygg_pid = None
    app.socks_pid = None
    if app.ygg_card.get_enable_expansion():
        app.ygg_card.set_enable_expansion(False)
    app.switch_row.set_subtitle("Stopped")
    app._set_ip_labels("-", "-")
    app._expand_ipv6_card(False)
    set_switch_lock(app, False)
    drain_pending(app)
    show_error_dialog(app, message)
    return False


def _monitor_process(app, pid: int, use_socks: bool) -> None:
    time.sleep(1)
    while True:
        current_pid = app.socks_pid if use_socks else app.ygg_pid
        if current_pid != pid:
            return

        is_running = Shell.is_alive(pid, as_root=not use_socks)
        if not is_running:
            current_pid = app.socks_pid if use_socks else app.ygg_pid
            if current_pid == pid:
                ygg_app = 'Yggstack' if use_socks else 'Yggdrasil'
                msg = f"The {ygg_app} process exited unexpectedly." 
                GLib.idle_add(on_process_error, app, msg)
            return

        time.sleep(2)


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
            ygg_app = 'Yggstack' if use_socks else 'Yggdrasil'
            GLib.idle_add(
                on_process_error,
                app,
                f"Failed to start {ygg_app}: {exc}",
            )
            return

        app.switch_row.set_subtitle("Running")
        app._set_ip_labels("-", "-")
        app._expand_ipv6_card(True)

        Thread(target=poll_for_addresses, args=(app, use_socks), daemon=True).start()
        Thread(target=_monitor_process, args=(app, pid, use_socks), daemon=True).start()

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


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")

