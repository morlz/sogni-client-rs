mod api;
mod cookies;
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
    cookies: Arc<ClearableCookieStore>,
    media: reqwest::Client,
}

impl HttpClients {
    pub(crate) fn build(request_timeout: Duration) -> Result<Self> {
        let user_agent = format!("sogni-client-rs/{}", crate::VERSION);
        let cookies = Arc::new(ClearableCookieStore::default());
        let authenticated = reqwest::Client::builder()
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(user_agent.clone())
            .build()?;
        let media = reqwest::Client::builder()
            .timeout(request_timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent(user_agent)
            .build()?;
        Ok(Self {
            authenticated,
            cookies,
            media,
        })
    }

    pub(crate) fn authenticated(&self) -> reqwest::Client {
        self.authenticated.clone()
    }

    pub(crate) fn cookies(&self) -> Arc<ClearableCookieStore> {
        self.cookies.clone()
    }
}
