from yggui.core.common import Runtime, Binary


def load_socks_config(app):
    cfg = Runtime.config.load()
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
    Runtime.config.set("yggstack-enable", state)
    app.socks_card.set_subtitle("Enabled" if state else "Disabled")
    app.socks_card.set_expanded(state)
    app.socks_config["enabled"] = state


def listen_changed(app, _row, _pspec):
    value = app.socks_listen_row.get_text().strip()
    if value:
        Runtime.config.set("yggstack-listen", value)
        app.socks_config["listen"] = value


def ip_changed(app, _row, _pspec):
    value = app.socks_dns_ip_row.get_text().strip()
    Runtime.config.set("yggstack-dns-ip", value)
    app.socks_config["dns_ip"] = value


def port_changed(app, _row, _pspec):
    value = app.socks_dns_port_row.get_text().strip() or "53"
    Runtime.config.set("yggstack-dns-port", value)
    app.socks_config["dns_port"] = value


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")

