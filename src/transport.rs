mod api;
mod cookies;
mod media_policy;
mod proxy;
mod rest;
mod socket;

#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Duration};

use crate::Result;

pub use api::ApiClient;
pub(crate) use cookies::ClearableCookieStore;
pub use rest::{RestClient, SseEvent, SseStream};

#[derive(Clone)]
pub(crate) struct HttpClients {
    authenticated: reqwest::Client,
    streaming: reqwest::Client,
    cookies: Arc<ClearableCookieStore>,
    media: reqwest::Client,
    strict_media: bool,
    media_proxy: Option<proxy::SocksProxy>,
}

impl HttpClients {
    pub(crate) fn build(request_timeout: Duration) -> Result<Self> {
        Self::configured(request_timeout, None, false)
    }

    pub(crate) fn build_with_proxy(request_timeout: Duration, proxy: &str) -> Result<Self> {
        let proxy = proxy::SocksProxy::parse(proxy)?;
        Self::configured(request_timeout, Some(proxy), false)
    }

    pub(crate) fn build_strict(request_timeout: Duration, proxy: Option<&str>) -> Result<Self> {
        Self::configured(
            request_timeout,
            proxy.map(proxy::SocksProxy::parse).transpose()?,
            true,
        )
    }

    fn configured(
        request_timeout: Duration,
        proxy: Option<proxy::SocksProxy>,
        strict_media: bool,
    ) -> Result<Self> {
        let user_agent = format!("sogni-client-rs/{}", crate::VERSION);
        let cookies = Arc::new(ClearableCookieStore::default());
        let mut authenticated = reqwest::Client::builder()
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(user_agent.clone());
        let mut media = reqwest::Client::builder()
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent(user_agent.clone());
        let mut streaming = reqwest::Client::builder()
            .connect_timeout(request_timeout)
            .read_timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(user_agent);
        if let Some(proxy) = &proxy {
            authenticated = authenticated.proxy(proxy.http_proxy()?);
            streaming = streaming.proxy(proxy.http_proxy()?);
            media = media.proxy(proxy.http_proxy()?);
        }
        let authenticated = authenticated.build()?;
        let media = media.build()?;
        Ok(Self {
            authenticated,
            streaming: streaming.build()?,
            cookies,
            media,
            strict_media,
            media_proxy: proxy,
        })
    }

    pub(crate) fn authenticated(&self) -> reqwest::Client {
        self.authenticated.clone()
    }

    pub(crate) fn cookies(&self) -> Arc<ClearableCookieStore> {
        self.cookies.clone()
    }
}
