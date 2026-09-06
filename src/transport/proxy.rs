use std::{borrow::Cow, net::SocketAddr};

use percent_encoding::percent_decode_str;
use tokio::net::TcpStream;
use tokio_socks::{TargetAddr, tcp::Socks5Stream};
use url::{Host, Url};

use crate::{Error, Result};

#[derive(Clone)]
pub(super) struct SocksProxy(Url);

impl SocksProxy {
    pub(super) fn parse(value: &str) -> Result<Self> {
        let url = Url::parse(value).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "socks5" | "socks5h")
            || url.host().is_none()
            || url.port() == Some(0)
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
            || (url.username().is_empty() && url.password().is_some())
        {
            return Err(invalid());
        }
        for value in [url.username(), url.password().unwrap_or_default()] {
            let decoded = percent_decode_str(value)
                .decode_utf8()
                .map_err(|_| invalid())?;
            if decoded.len() > 255 || decoded.contains('\0') {
                return Err(invalid());
            }
        }
        Ok(Self(url))
    }

    pub(super) fn http_proxy(&self) -> Result<reqwest::Proxy> {
        reqwest::Proxy::all(self.0.as_str()).map_err(|_| invalid())
    }

    pub(super) fn pinned_http_proxy(&self) -> Result<reqwest::Proxy> {
        let mut local = self.0.clone();
        local.set_scheme("socks5").map_err(|()| invalid())?;
        reqwest::Proxy::all(local).map_err(|_| invalid())
    }

    pub(super) async fn connect(&self, destination: &Url) -> Result<TcpStream> {
        let target = self.target(destination).await?;
        let host = self
            .0
            .host_str()
            .ok_or_else(invalid)?
            .trim_matches(['[', ']']);
        let address = (host, self.0.port().unwrap_or(1080));
        let stream = if self.0.username().is_empty() {
            Socks5Stream::connect(address, target).await
        } else {
            let username = percent_decode_str(self.0.username())
                .decode_utf8()
                .map_err(|_| invalid())?;
            let password = percent_decode_str(self.0.password().unwrap_or_default())
                .decode_utf8()
                .map_err(|_| invalid())?;
            Socks5Stream::connect_with_password(address, target, &username, &password).await
        }
        .map_err(|_| Error::Transport("SOCKS proxy connection failed".into()))?;
        Ok(stream.into_inner())
    }

    async fn target<'a>(&self, destination: &'a Url) -> Result<TargetAddr<'a>> {
        let port = destination.port_or_known_default().ok_or_else(invalid)?;
        let host = destination.host().ok_or_else(invalid)?;
        match host {
            Host::Ipv4(ip) => Ok(TargetAddr::Ip(SocketAddr::new(ip.into(), port))),
            Host::Ipv6(ip) => Ok(TargetAddr::Ip(SocketAddr::new(ip.into(), port))),
            Host::Domain(host) if self.0.scheme() == "socks5h" => {
                Ok(TargetAddr::Domain(Cow::Borrowed(host), port))
            }
            Host::Domain(host) => {
                let address = tokio::net::lookup_host((host, port))
                    .await
                    .map_err(|_| Error::Transport("proxy target lookup failed".into()))?
                    .next()
                    .ok_or_else(|| Error::Transport("proxy target lookup empty".into()))?;
                Ok(TargetAddr::Ip(address))
            }
        }
    }
}

fn invalid() -> Error {
    Error::InvalidInput("invalid SOCKS5 proxy configuration".into())
}

#[cfg(test)]
mod tests;
