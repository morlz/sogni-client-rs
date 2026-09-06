use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use url::Url;

use super::proxy::SocksProxy;
use crate::{Error, Result};

const MAX_DNS_ADDRESSES: usize = 64;

pub(super) async fn client(
    url: &Url,
    timeout: Duration,
    proxy: Option<&SocksProxy>,
) -> Result<reqwest::Client> {
    let host = safe_host(url)?;
    if timeout.is_zero() {
        return Err(invalid());
    }
    let addresses = tokio::time::timeout(timeout, tokio::net::lookup_host((host, 443)))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
        .take(MAX_DNS_ADDRESSES + 1)
        .collect::<Vec<_>>();
    validate_addresses(&addresses)?;

    pinned_client(host, &addresses, timeout, proxy)
}

fn validate_addresses(addresses: &[SocketAddr]) -> Result<()> {
    if addresses.is_empty()
        || addresses.len() > MAX_DNS_ADDRESSES
        || addresses
            .iter()
            .any(|address| !public_address(address.ip()))
    {
        return Err(invalid());
    }

    Ok(())
}

fn pinned_client(
    host: &str,
    addresses: &[SocketAddr],
    timeout: Duration,
    proxy: Option<&SocksProxy>,
) -> Result<reqwest::Client> {
    // This client has no API authentication headers or cookie store. Keep the
    // original URL host for TLS verification while fixing its approved addresses.
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout)
        .resolve_to_addrs(host, addresses);
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy.pinned_http_proxy()?);
    }

    builder.build().map_err(|_| unavailable())
}

fn safe_host(url: &Url) -> Result<&str> {
    let host = url.host_str().ok_or_else(invalid)?;
    let allowed = ["sogni.ai", "sogni.io", "amazonaws.com"]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
    if !allowed
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }

    Ok(host)
}

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_v4(ip);
            }
            let segments = ip.segments();

            (0x2000..0x4000).contains(&segments[0])
                && !matches!(segments[0], 0x2002 | 0x3fff)
                && (segments[0] != 0x2001 || (segments[1] >= 0x0200 && segments[1] != 0x0db8))
        }
    }
}

const fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();

    !(matches!(a, 0 | 10 | 127 | 224..=255)
        || a == 100 && b >= 64 && b <= 127
        || a == 169 && b == 254
        || a == 172 && b >= 16 && b <= 31
        || a == 192 && (b == 168 || (b == 0 && c <= 2))
        || a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100))
        || a == 203 && b == 0 && c == 113)
}

fn invalid() -> Error {
    Error::InvalidInput("media destination is not permitted".into())
}

fn unavailable() -> Error {
    Error::Transport("media destination is unavailable".into())
}

#[cfg(test)]
#[path = "media_policy_tests.rs"]
mod tests;
