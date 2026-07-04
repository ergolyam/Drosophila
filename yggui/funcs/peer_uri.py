from urllib.parse import parse_qs, urlencode


def build_peer_uri(
    protocol: str,
    domain: str,
    *,
    sni: str = "",
    password: str = "",
) -> str:
    query: list[tuple[str, str]] = []
    if protocol == "tls" and sni:
        query.append(("sni", sni))
    if password:
        query.append(("password", password))

    peer = f"{protocol}://{domain}"
    if query:
        peer += f"?{urlencode(query)}"
    return peer


def peer_query_value(query: str, key: str) -> str | None:
    return parse_qs(query).get(key, [None])[0]


def peer_subtitle_parts(protocol: str, query: str) -> list[str]:
    parts = [protocol.upper()]

    sni = peer_query_value(query, "sni")
    if protocol == "tls" and sni:
        parts.append(f"SNI: {sni}")

    if peer_query_value(query, "password"):
        parts.append("Password")

    return parts
