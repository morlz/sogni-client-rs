use super::*;

/// Settings for `AccountApi::login_with_settings`. Restoration is opt-in;
/// repeat the original settings after the person confirms restoring an account.
#[derive(Clone, Debug, Default)]
pub struct LoginOptions {
    pub remember_me: bool,
    pub app_source: Option<String>,
    /// Restore a deleted account and sign in. False omits the wire flag.
    pub restore_deleted_account: bool,
}

impl AccountApi {
    pub async fn get_nonce(&self, wallet_address: &str) -> Result<String> {
        if wallet_address.trim().is_empty() {
            return Err(Error::InvalidInput("wallet_address is required".into()));
        }
        let response = self
            .client
            .rest
            .post(
                "/v1/account/nonce",
                &json!({"walletAddress": wallet_address}),
            )
            .await?;
        data(&response)
            .get("nonce")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::Protocol("nonce response did not include data.nonce".into()))
    }

    #[cfg(feature = "wallet")]
    pub async fn login(&self, username: &str, password: &str) -> Result<Value> {
        self.login_with_options(username, password, false, None)
            .await
    }

    #[cfg(feature = "wallet")]
    pub async fn login_with_options(
        &self,
        username: &str,
        password: &str,
        remember_me: bool,
        app_source: Option<&str>,
    ) -> Result<Value> {
        self.login_with_settings(
            username,
            password,
            LoginOptions {
                remember_me,
                app_source: app_source.map(ToOwned::to_owned),
                ..Default::default()
            },
        )
        .await
    }

    /// Sign in with explicit settings. A restorable account returns an
    /// [`crate::ApiError`] with [`crate::ACCOUNT_RESTORABLE_ERROR_CODE`] and
    /// `details().deletedAt` (Unix seconds). No automatic retry or restoration
    /// occurs: set `restore_deleted_account` only after the person agrees.
    #[cfg(feature = "wallet")]
    pub async fn login_with_settings(
        &self,
        username: &str,
        password: &str,
        options: LoginOptions,
    ) -> Result<Value> {
        let session = self.client.rest.request_session();
        let wallet = wallet(username, password)?;
        let address = format!("{:#x}", wallet.address());
        let nonce = session.run(self.get_nonce(&address)).await?;
        let signature = session
            .run(self.sign_named(
                &wallet,
                "Authentication",
                json!([{"name":"walletAddress","type":"address"},{"name":"nonce","type":"string"}]),
                json!({"walletAddress":address,"nonce":nonce}),
            ))
            .await?;
        let mut body =
            json!({"walletAddress":address,"signature":signature,"rememberMe":options.remember_me});
        if let Some(source) = options
            .app_source
            .as_deref()
            .or_else(|| self.client.app_source())
        {
            body["appSource"] = json!(source);
        }
        if options.restore_deleted_account {
            body["restoreDeletedAccount"] = json!(true);
        }
        let response = session
            .run(self.client.rest.post("/v1/account/login", &body))
            .await?;
        let payload = data(&response).clone();
        self.authenticate_response_and_hydrate(&payload, &address, &session)
            .await?;
        self.client.start().await?;
        Ok(payload)
    }
}
