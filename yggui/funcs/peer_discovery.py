import base64, json, os, socket, ssl, time
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from threading import Lock, Thread
from urllib.request import urlopen
from urllib.error import URLError
from urllib.parse import urlparse

from gi.repository import Gtk, Adw, GLib  # type: ignore

from yggui.core.common import Gui


@dataclass
class DiscoveredPeer:
    address: str
    protocol: str
    host: str
    port: int
    country: str
    response_ms: int
    ping_ms: int | None = None

    def display_ping(self) -> str:
        if self.ping_ms is None:
            return "-"
        return f"{self.ping_ms} ms"


class PeerDiscoveryDialog:
    def __init__(self, app, on_selected):
        self.app = app
        self.on_selected = on_selected
        self.builder = Gtk.Builder.new_from_file(str(Gui.peer_discovery_ui_file))

        self.dialog: Adw.AlertDialog = self.builder.get_object("peer_discovery_dialog")
        self.list_box: Gtk.ListBox = self.builder.get_object("peer_discovery_list")
        self.sort_row: Adw.ComboRow = self.builder.get_object("peer_sort_row")
        self.progress_box: Gtk.Box = self.builder.get_object("peer_discovery_progress_box")
        self.progress_label: Gtk.Label = self.builder.get_object("peer_discovery_progress_label")
        self.progress_spinner: Gtk.Spinner = self.builder.get_object("peer_discovery_spinner")
        self.refresh_btn: Gtk.Button = self.builder.get_object("refresh_peers_btn")

        self.filter_buttons = {
            "tcp": self.builder.get_object("filter_tcp"),
            "tls": self.builder.get_object("filter_tls"),
            "ws": self.builder.get_object("filter_ws"),
            "wss": self.builder.get_object("filter_wss"),
            "quic": self.builder.get_object("filter_quic"),
        }

        self._urls = [
            "https://publicpeers.neilalexander.dev/publicnodes.json",
            "https://peers.yggdrasil.link/publicnodes.json",
        ]
        self._default_protocols = {"tcp", "tls"}
        self._enabled_protocols = set(self._default_protocols)
        self._loaded_protocols = set()
        self._all_peers_raw = None
        self._peers_by_address: dict[str, DiscoveredPeer] = {}
        self._peers_lock = Lock()
        self._search_generation = 0
        self._search_progress = {"total": 0, "checked": 0, "available": 0}
        self._selected_address: str | None = None
        self._max_workers = 20
        self._check_timeout = 3.0
        self._refresh_source_id: int | None = None

        self._init_ui()
        self._load_cache()

    def present(self) -> None:
        self.dialog.present(self.app.win)
        self._start_search(self._get_missing_protocols())

    def _init_ui(self) -> None:
        self.dialog.set_response_enabled("use", False)

        for proto, btn in self.filter_buttons.items():
            btn.set_active(proto in self._default_protocols)
            btn.connect("toggled", lambda _b, p=proto: self._on_filter_toggled(p))

        self.sort_row.connect("notify::selected", self._on_sort_changed)
        self.list_box.connect("row-selected", self._on_row_selected)
        self.refresh_btn.connect("clicked", self._on_refresh_clicked)

        self.dialog.connect("response", self._on_response)

    def _load_cache(self) -> None:
        cache = getattr(self.app, "_peer_discovery_cache", None)
        if not cache:
            return
        peers = cache.get("peers", {})
        loaded = cache.get("loaded_protocols", set())
        enabled = cache.get("enabled_protocols")
        if enabled:
            self._enabled_protocols = set(enabled)
            for proto, btn in self.filter_buttons.items():
                btn.set_active(proto in self._enabled_protocols)
        if peers:
            self._peers_by_address = dict(peers)
            self._loaded_protocols = set(loaded)
            self._refresh_list()
            self._show_cached_count()

    def _show_cached_count(self) -> None:
        total_peers = len(self._peers_by_address)
        if total_peers == 0:
            return
        self.progress_label.set_label(f"Shown {len(self._get_visible_peers())} peers")
        self.progress_spinner.set_spinning(False)
        self.progress_spinner.set_visible(False)
        self.progress_box.set_visible(True)

    def _save_cache(self) -> None:
        self.app._peer_discovery_cache = {
            "peers": dict(self._peers_by_address),
            "loaded_protocols": set(self._loaded_protocols),
            "enabled_protocols": set(self._enabled_protocols),
        }

    def _get_missing_protocols(self) -> list[str]:
        return [p for p in self._enabled_protocols if p not in self._loaded_protocols]

    def _on_response(self, _dialog, response: str) -> None:
        if response != "use":
            return
        peer = self._peers_by_address.get(self._selected_address or "")
        if peer is None:
            return
        self.on_selected(peer)

    def _on_row_selected(self, _listbox, row) -> None:
        if row is None:
            self._selected_address = None
            self.dialog.set_response_enabled("use", False)
            return
        self._selected_address = getattr(row, "peer_address", None)
        self.dialog.set_response_enabled("use", self._selected_address is not None)

    def _on_sort_changed(self, _row, _pspec) -> None:
        self._refresh_list()

    def _on_refresh_clicked(self, _btn) -> None:
        self.app._peer_discovery_cache = None
        self._search_generation += 1
        self._peers_by_address = {}
        self._loaded_protocols = set()
        self._search_progress = {"total": 0, "checked": 0, "available": 0}
        self._selected_address = None
        self.dialog.set_response_enabled("use", False)
        self._refresh_list()
        self._update_progress_label()
        self._start_search(list(self._enabled_protocols), refresh=True)

    def _on_filter_toggled(self, proto: str) -> None:
        btn = self.filter_buttons[proto]
        if btn.get_active():
            self._enabled_protocols.add(proto)
            if proto not in self._loaded_protocols:
                self._start_search([proto])
        else:
            if proto in self._enabled_protocols:
                if len(self._enabled_protocols) <= 1:
                    btn.set_active(True)
                    return
                self._enabled_protocols.remove(proto)
        self._refresh_list()
        self._save_cache()

    def _set_progress_visible(self, visible: bool) -> None:
        self.progress_box.set_visible(visible)
        self.progress_spinner.set_spinning(visible)
        self.progress_spinner.set_visible(visible)

    def _update_progress_label(self) -> None:
        total = self._search_progress["total"]
        checked = self._search_progress["checked"]
        available = self._search_progress["available"]
        if total > 0:
            self.progress_label.set_label(
                f"Searching: {checked}/{total} (found {available})"
            )
        else:
            self.progress_label.set_label("Searching for peers…")

    def _start_search(self, protocols: list[str], refresh: bool = False) -> None:
        if not protocols:
            return
        self._set_progress_visible(True)
        self.progress_label.set_label("Fetching peer list…")
        self._set_filter_buttons_sensitive(False)
        self.refresh_btn.set_sensitive(False)

        def _run_search(generation: int, wanted_protocols: list[str]) -> None:
            try:
                raw = self._fetch_peers_raw(refresh=refresh)
            except Exception:
                GLib.idle_add(self._show_no_peers)
                return

            peers = self._build_peers(raw, wanted_protocols)
            if not peers:
                GLib.idle_add(self._mark_protocols_loaded, wanted_protocols)
                GLib.idle_add(self._show_no_peers)
                return

            GLib.idle_add(self._mark_search_total, len(peers))

            available = self._check_peers(peers, generation)
            if generation != self._search_generation:
                return
            GLib.idle_add(self._mark_protocols_loaded, wanted_protocols)
            if available == 0:
                GLib.idle_add(self._show_no_peers)

        Thread(
            target=_run_search,
            args=(self._search_generation, protocols),
            daemon=True,
        ).start()

    def _mark_search_total(self, count: int) -> None:
        self._search_progress["total"] += count
        self._set_progress_visible(True)
        self._update_progress_label()

    def _mark_peer_checked(self, available: bool) -> None:
        self._search_progress["checked"] += 1
        if available:
            self._search_progress["available"] += 1
        self._update_progress_label()

    def _finish_progress(self) -> None:
        if self._search_progress["checked"] >= self._search_progress["total"]:
            visible_count = len(self._get_visible_peers())
            if visible_count == 0:
                self.progress_label.set_label("No peers available")
            else:
                self.progress_label.set_label(f"Shown {visible_count} peers")
            self.progress_spinner.set_spinning(False)
            self.progress_spinner.set_visible(False)
            self.progress_box.set_visible(True)
            self._set_filter_buttons_sensitive(True)
            self.refresh_btn.set_sensitive(True)

    def _show_no_peers(self) -> None:
        self.progress_label.set_label("No peers available")
        self.progress_spinner.set_spinning(False)
        self.progress_spinner.set_visible(False)
        self.progress_box.set_visible(True)
        self._set_filter_buttons_sensitive(True)
        self.refresh_btn.set_sensitive(True)

    def _fetch_peers_raw(self, refresh: bool) -> dict:
        if self._all_peers_raw is not None and not refresh:
            return self._all_peers_raw

        last_error = None
        if refresh:
            self._all_peers_raw = None
        for url in self._urls:
            try:
                with urlopen(url, timeout=10) as resp:
                    data = json.loads(resp.read().decode("utf-8"))
                    self._all_peers_raw = data
                    return data
            except (URLError, OSError, json.JSONDecodeError) as err:
                last_error = err

        if last_error is not None:
            raise last_error
        raise RuntimeError("Failed to fetch peers list")

    def _build_peers(self, raw: dict, protocols: list[str]) -> list[DiscoveredPeer]:
        wanted = set(protocols)
        peers: list[DiscoveredPeer] = []

        for region, entries in raw.items():
            country = region.replace(".md", "")
            for address, meta in entries.items():
                parsed = urlparse(address)
                proto = parsed.scheme
                if proto not in wanted:
                    continue
                if not parsed.hostname or not parsed.port:
                    continue

                response_ms = int(meta.get("response_ms", 0))
                peer = DiscoveredPeer(
                    address=address,
                    protocol=proto,
                    host=parsed.hostname,
                    port=parsed.port,
                    country=country,
                    response_ms=response_ms,
                )
                peers.append(peer)

        peers.sort(key=lambda p: p.response_ms or 0)
        return peers

    def _check_peers(self, peers: list[DiscoveredPeer], generation: int) -> int:
        available_count = 0
        with ThreadPoolExecutor(max_workers=self._max_workers) as executor:
            futures = {}
            for peer in peers:
                futures[executor.submit(self._check_peer, peer)] = peer
            for future in as_completed(futures):
                peer = futures[future]
                if generation != self._search_generation:
                    return available_count
                try:
                    available = future.result()
                except Exception:
                    available = False
                GLib.idle_add(self._mark_peer_checked, available)
                if available:
                    available_count += 1
                    GLib.idle_add(self._add_peer, peer, generation)

        GLib.idle_add(self._finish_progress)
        return available_count

    def _mark_protocols_loaded(self, protocols: list[str]) -> None:
        self._loaded_protocols.update(protocols)
        self._save_cache()

    def _check_peer(self, peer: DiscoveredPeer) -> bool:
        if peer.protocol == "quic":
            peer.ping_ms = peer.response_ms if peer.response_ms > 0 else None
            return True

        checkers = {
            "tcp": self._check_tcp,
            "tls": self._check_tls,
            "ws": self._check_ws,
            "wss": self._check_wss,
        }
        checker = checkers.get(peer.protocol)
        if checker is None:
            return False
        return checker(peer)

    def _check_tcp(self, peer: DiscoveredPeer) -> bool:
        start = time.perf_counter()
        try:
            with socket.create_connection(
                (peer.host, peer.port), timeout=self._check_timeout
            ):
                peer.ping_ms = int((time.perf_counter() - start) * 1000)
                return True
        except OSError:
            return False

    def _check_tls(self, peer: DiscoveredPeer) -> bool:
        start = time.perf_counter()
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        try:
            with socket.create_connection(
                (peer.host, peer.port), timeout=self._check_timeout
            ) as sock:
                with context.wrap_socket(sock, server_hostname=peer.host):
                    peer.ping_ms = int((time.perf_counter() - start) * 1000)
                    return True
        except OSError:
            return False

    def _check_ws(self, peer: DiscoveredPeer) -> bool:
        return self._check_websocket(peer, use_tls=False)

    def _check_wss(self, peer: DiscoveredPeer) -> bool:
        return self._check_websocket(peer, use_tls=True)

    def _check_websocket(self, peer: DiscoveredPeer, use_tls: bool) -> bool:
        start = time.perf_counter()
        try:
            with socket.create_connection(
                (peer.host, peer.port), timeout=self._check_timeout
            ) as raw_sock:
                raw_sock.settimeout(self._check_timeout)
                if use_tls:
                    context = ssl.create_default_context()
                    context.check_hostname = False
                    context.verify_mode = ssl.CERT_NONE
                    with context.wrap_socket(raw_sock, server_hostname=peer.host) as tls_sock:
                        return self._send_ws_request(peer, tls_sock, start)
                return self._send_ws_request(peer, raw_sock, start)
        except OSError:
            return False

    def _send_ws_request(self, peer: DiscoveredPeer, sock: socket.socket, start: float) -> bool:
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        request = (
            "GET / HTTP/1.1\r\n"
            f"Host: {peer.host}:{peer.port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        sock.sendall(request.encode("ascii"))
        response = sock.recv(256)
        if b" 101 " not in response and b" 101\r" not in response:
            return False
        peer.ping_ms = int((time.perf_counter() - start) * 1000)
        return True

    def _add_peer(self, peer: DiscoveredPeer, generation: int) -> None:
        if generation != self._search_generation:
            return
        if peer.protocol not in self._enabled_protocols:
            return
        with self._peers_lock:
            if peer.address in self._peers_by_address:
                return
            self._peers_by_address[peer.address] = peer
            self._loaded_protocols.add(peer.protocol)
            self._save_cache()
        self._schedule_refresh()

    def _schedule_refresh(self) -> None:
        if self._refresh_source_id is not None:
            return

        def _run_refresh() -> bool:
            self._refresh_source_id = None
            self._refresh_list()
            return False

        self._refresh_source_id = GLib.timeout_add(200, _run_refresh)

    def _set_filter_buttons_sensitive(self, sensitive: bool) -> None:
        for btn in self.filter_buttons.values():
            btn.set_sensitive(sensitive)

    def _refresh_list(self) -> None:
        peers = self._get_visible_peers()
        current = self.list_box.get_first_child()
        while current:
            nxt = current.get_next_sibling()
            self.list_box.remove(current)
            current = nxt

        for peer in peers:
            row = Adw.ActionRow()
            row.set_title(f"{peer.host}:{peer.port}")
            row.set_subtitle(f"{peer.protocol.upper()} • {peer.country}")
            row.set_activatable(True)
            row.peer_address = peer.address

            ping_label = Gtk.Label()
            ping_label.set_label(peer.display_ping())
            ping_label.set_xalign(1.0)
            row.add_suffix(ping_label)

            self.list_box.append(row)

        if self._selected_address:
            row = self.list_box.get_first_child()
            while row:
                if getattr(row, "peer_address", None) == self._selected_address:
                    self.list_box.select_row(row)
                    break
                row = row.get_next_sibling()

        if not self.progress_spinner.get_spinning():
            if len(peers) == 0:
                self.progress_label.set_label("No peers available")
            else:
                self.progress_label.set_label(f"Shown {len(peers)} peers")
            self.progress_box.set_visible(True)


    def _get_visible_peers(self) -> list[DiscoveredPeer]:
        peers = [
            peer
            for peer in self._peers_by_address.values()
            if peer.protocol in self._enabled_protocols
        ]

        sort_selected = self.sort_row.get_selected()
        if sort_selected == 1:
            peers.sort(key=lambda p: (p.country.lower(), p.ping_ms or 999999))
        else:
            peers.sort(key=lambda p: (p.ping_ms or 999999, p.country.lower()))
        return peers


def open_peer_discovery_dialog(app, on_selected) -> None:
    dialog = PeerDiscoveryDialog(app, on_selected)
    dialog.present()


if __name__ == "__main__":
    raise RuntimeError("This module should be run only via main.py")
