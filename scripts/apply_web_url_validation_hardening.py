from pathlib import Path

path = Path("src-tauri/src/inference.rs")
text = path.read_text(encoding="utf-8")

old_import = 'use std::{collections::HashMap, sync::Mutex, time::Instant};'
new_import = '''use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::Mutex,
    time::Instant,
};'''
if old_import not in text:
    raise SystemExit("expected std import not found")
text = text.replace(old_import, new_import, 1)

old_fn = '''fn is_public_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return false;
    }
    let host = lower
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', ':', '?', '#']).next().unwrap_or(""))
        .unwrap_or("");
    !matches!(host, "localhost" | "0.0.0.0" | "127.0.0.1" | "::1")
        && !host.starts_with("10.")
        && !host.starts_with("192.168.")
        && !host.starts_with("169.254.")
        && !host.starts_with("172.16.")
        && !host.starts_with("172.17.")
        && !host.starts_with("172.18.")
        && !host.starts_with("172.19.")
        && !host.starts_with("172.2")
        && !host.starts_with("172.30.")
        && !host.starts_with("172.31.")
}
'''
new_fn = '''fn is_public_web_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    // URL host strings may retain IPv6 brackets depending on the URL backend.
    // Normalize those before IpAddr parsing so loopback/private IPv6 cannot be
    // misclassified as a public domain name.
    let normalized_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized_host == "localhost"
        || normalized_host.ends_with(".localhost")
        || normalized_host.ends_with(".local")
    {
        return false;
    }
    match normalized_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => is_public_ipv4(address),
        Ok(IpAddr::V6(address)) => is_public_ipv6(address),
        Err(_) => true,
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _fourth] = address.octets();
    if address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_multicast()
        || address == Ipv4Addr::BROADCAST
    {
        return false;
    }

    // Additional non-public ranges not covered by the standard helpers.
    if first == 0
        || (first == 100 && (64..=127).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 192 && second == 0 && third == 2)
        || (first == 198 && (second == 18 || second == 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113)
        || first >= 240
    {
        return false;
    }
    true
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
        return false;
    }
    let segments = address.segments();
    // fc00::/7 unique-local and fe80::/10 link-local.
    if (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80 {
        return false;
    }

    // Reject IPv4-compatible/mapped IPv6 when the embedded IPv4 is non-public.
    if segments[..5] == [0, 0, 0, 0, 0] && matches!(segments[5], 0 | 0xffff) {
        let high = segments[6].to_be_bytes();
        let low = segments[7].to_be_bytes();
        let embedded = Ipv4Addr::new(high[0], high[1], low[0], low[1]);
        return is_public_ipv4(embedded);
    }
    true
}
'''
if old_fn not in text:
    raise SystemExit("expected is_public_web_url implementation not found")
text = text.replace(old_fn, new_fn, 1)

old_test = '''    #[test]
    fn rejects_private_search_urls() {
        assert!(!is_public_web_url("http://127.0.0.1/admin"));
        assert!(!is_public_web_url("http://192.168.1.20/"));
        assert!(is_public_web_url("https://example.com/article"));
    }
'''
new_test = '''    #[test]
    fn rejects_private_and_local_search_urls() {
        assert!(!is_public_web_url("http://127.0.0.1/admin"));
        assert!(!is_public_web_url("http://10.0.0.1/"));
        assert!(!is_public_web_url("http://172.16.0.1/"));
        assert!(!is_public_web_url("http://172.31.255.255/"));
        assert!(!is_public_web_url("http://192.168.1.20/"));
        assert!(!is_public_web_url("http://169.254.1.1/"));
        assert!(!is_public_web_url("http://[::1]/"));
        assert!(!is_public_web_url("http://[fc00::1]/"));
        assert!(!is_public_web_url("http://[fd12::1]/"));
        assert!(!is_public_web_url("http://[fe80::1]/"));
        assert!(!is_public_web_url("http://printer.local/"));
        assert!(!is_public_web_url("file:///tmp/test"));
    }

    #[test]
    fn allows_public_search_urls_without_prefix_false_positives() {
        assert!(is_public_web_url("https://172.2.1.1/"));
        assert!(is_public_web_url("https://172.32.0.1/"));
        assert!(is_public_web_url("https://example.com/article"));
        assert!(is_public_web_url("http://8.8.8.8/"));
    }
'''
if old_test not in text:
    raise SystemExit("expected search URL test not found")
text = text.replace(old_test, new_test, 1)

path.write_text(text, encoding="utf-8")
