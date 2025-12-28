import json
from yggui.core.common import Runtime, Binary


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


def save_param(key: str, value):
    cfg = read_config()
    cfg[key] = value
    write_config(cfg)


def load_socks_config(app):
    cfg = read_config()
    enabled = cfg.get("yggstack-enable", False)
    listen = cfg.get("yggstack-listen", "127.0.0.1:1080")
    dns_ip = cfg.get("yggstack-dns-ip", "")
    dns_port = cfg.get("yggstack-dns-port", "53")
    if Binary.yggstack_path is None:
        enabled = False
        app.socks_card.set_sensitive(False)
        app.socks_card.set_subtitle("Yggstack not found")
    app.socks_config = {
        "enabled": enabled,
        "listen": listen,
        "dns_ip": dns_ip,
        "dns_port": dns_port,
    }
    app.socks_card.set_enable_expansion(enabled)
    app.socks_card.set_subtitle("Enabled" if enabled else "Disabled")
    app.socks_listen_row.set_text(listen)
    app.socks_dns_ip_row.set_text(dns_ip)
    app.socks_dns_port_row.set_text(dns_port)
    app.socks_card.set_expanded(enabled)


def socks_switch_toggled(app, _row, state: bool):
    save_param("yggstack-enable", state)
    app.socks_card.set_subtitle("Enabled" if state else "Disabled")
    app.socks_card.set_expanded(state)
    app.socks_config["enabled"] = state


def listen_changed(app, _row, _pspec):
    value = app.socks_listen_row.get_text().strip()
    if value:
        save_param("yggstack-listen", value)
        app.socks_config["listen"] = value


def ip_changed(app, _row, _pspec):
    value = app.socks_dns_ip_row.get_text().strip()
    save_param("yggstack-dns-ip", value)
    app.socks_config["dns_ip"] = value


def port_changed(app, _row, _pspec):
    value = app.socks_dns_port_row.get_text().strip() or "53"
    save_param("yggstack-dns-port", value)
    app.socks_config["dns_port"] = value


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")

