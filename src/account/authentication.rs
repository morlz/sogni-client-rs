use parking_lot::Mutex;
use serde_json::{Value, json};

#[cfg(feature = "wallet")]
use super::wallet_crypto::wallet;
use super::{AccountApi, AuthProjectionState, CurrentAccount, SubscriptionProjectionState, data};
#[cfg(feature = "wallet")]
use crate::AuthKind;
use crate::{Error, Result, transport::ApiClient};

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
        use ethers_signers::Signer as _;

        let wallet = wallet(username, password)?;
        let address = format!("{:#x}", wallet.address());
        let nonce = self.get_nonce(&address).await?;
        let signature = self
            .sign_named(
                &wallet,
                "Authentication",
                json!([
                    {"name": "walletAddress", "type": "address"},
                    {"name": "nonce", "type": "string"}
                ]),
                json!({"walletAddress": address, "nonce": nonce}),
            )
            .await?;
        let mut body = json!({
            "walletAddress": address,
            "signature": signature,
            "rememberMe": remember_me,
        });
        if let Some(source) = app_source.or_else(|| self.client.app_source()) {
            body["appSource"] = json!(source);
        }
        let response = self.client.rest.post("/v1/account/login", &body).await?;
        let payload = data(&response).clone();
        self.authenticate_response_and_hydrate(&payload).await?;
        self.client.start().await?;
        Ok(payload)
    }

    #[cfg(feature = "wallet")]
    pub async fn create_account(
        &self,
        username: &str,
        email: &str,
        password: &str,
        options: Value,
    ) -> Result<Value> {
        use ethers_signers::Signer as _;

        if username.is_empty() || email.is_empty() || password.is_empty() {
            return Err(Error::InvalidInput(
                "username, email, and password are required".into(),
            ));
        }
        let wallet = wallet(username, password)?;
        let address = format!("{:#x}", wallet.address());
        let nonce = self.get_nonce(&address).await?;
        let subscribe = options
            .get("subscribe")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut body = json!({
            "appid": self.client.app_id(),
            "username": username,
            "email": email,
            "subscribe": i32::from(subscribe),
            "walletAddress": address,
            "turnstileToken": options.get("turnstileToken"),
            "rememberMe": options.get("rememberMe").and_then(Value::as_bool).unwrap_or(false),
        });
        let signature = self
            .sign_named(
                &wallet,
                "Signup",
                json!([
                    {"name": "appid", "type": "string"},
                    {"name": "username", "type": "string"},
                    {"name": "email", "type": "string"},
                    {"name": "subscribe", "type": "uint256"},
                    {"name": "walletAddress", "type": "address"},
                    {"name": "nonce", "type": "string"}
                ]),
                json!({
                    "appid": self.client.app_id(),
                    "username": username,
                    "email": email,
                    "subscribe": i32::from(subscribe),
                    "walletAddress": address,
                    "nonce": nonce,
                }),
            )
            .await?;
        body["signature"] = json!(signature);
        if let Some(value) = options.get("referralCode") {
            body["referralCode"] = value.clone();
        }
        if let Some(source) = options
            .get("appSource")
            .and_then(Value::as_str)
            .or_else(|| self.client.app_source())
        {
            body["appSource"] = json!(source);
        }
        let response = self.client.rest.post("/v1/account/create", &body).await?;
        let payload = data(&response).clone();
        self.authenticate_response_and_hydrate(&payload).await?;
        self.client.start().await?;
        Ok(payload)
    }

    #[cfg(feature = "wallet")]
    async fn authenticate_response_and_hydrate(&self, payload: &Value) -> Result<()> {
        let mut projection = self.auth_projection.lock().await;
        projection.skip_next_authenticated_update = self.client.auth_kind() != AuthKind::ApiKey;
        if let Err(error) = self.authenticate_response(payload).await {
            projection.skip_next_authenticated_update = false;
            if !self.client.is_authenticated() {
                projection.hydrated = false;
                self.reset_account_projection();
            }
            return Err(error);
        }
        projection.hydrated = false;
        self.reset_account_projection_for_authentication();
        let account = match fetch_current_account(&self.client).await {
            Ok(account) => account,
            Err(error) => {
                self.rollback_failed_initial_hydration(
                    &mut projection,
                    self.client.auth_kind() != AuthKind::ApiKey,
                );
                return Err(error);
            }
        };
        apply_current_account(&self.current, &account);
        projection.hydrated = true;
        Ok(())
    }

    #[cfg(feature = "wallet")]
    async fn authenticate_response(&self, payload: &Value) -> Result<()> {
        match self.client.auth_kind() {
            AuthKind::Token => {
                let token = payload
                    .get("token")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Protocol("authentication response has no token".into())
                    })?;
                let refresh = payload
                    .get("refreshToken")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Protocol("authentication response has no refreshToken".into())
                    })?;
                self.client.set_tokens(token.into(), refresh.into()).await
            }
            AuthKind::Cookies => self.client.authenticate_cookies(),
            AuthKind::ApiKey => Ok(()),
        }
    }

    pub async fn logout(&self) -> Result<()> {
        match self
            .client
            .rest
            .post("/v1/account/logout", &json!({}))
            .await
        {
            Err(Error::Api(error)) if error.status == 401 => {}
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        self.client.clear_auth();
        self.clear_deauthenticated_projection().await;
        Ok(())
    }

    pub async fn me(&self) -> Result<Value> {
        let mut projection = self.auth_projection.lock().await;
        let payload = fetch_current_account(&self.client).await?;
        apply_current_account(&self.current, &payload);
        projection.hydrated = true;
        Ok(payload)
    }

    pub(crate) async fn hydrate_authenticated_projection(&self) -> Result<()> {
        let mut projection = self.auth_projection.lock().await;
        if projection.hydrated || !self.client.is_authenticated() {
            return Ok(());
        }
        let payload = fetch_current_account(&self.client).await?;
        if !self.client.is_authenticated() {
            return Ok(());
        }
        apply_current_account(&self.current, &payload);
        projection.hydrated = true;
        Ok(())
    }

    pub(crate) async fn set_tokens_and_hydrate(
        &self,
        token: String,
        refresh_token: String,
    ) -> Result<()> {
        let mut projection = self.auth_projection.lock().await;
        projection.skip_next_authenticated_update = true;
        if let Err(error) = self.client.set_tokens(token, refresh_token).await {
            projection.skip_next_authenticated_update = false;
            if !self.client.is_authenticated() {
                projection.hydrated = false;
                self.reset_account_projection();
            }
            return Err(error);
        }
        projection.hydrated = false;
        self.reset_account_projection_for_authentication();
        let payload = match fetch_current_account(&self.client).await {
            Ok(payload) => payload,
            Err(error) => {
                self.rollback_failed_initial_hydration(&mut projection, true);
                return Err(error);
            }
        };
        apply_current_account(&self.current, &payload);
        projection.hydrated = true;
        Ok(())
    }

    pub(crate) async fn clear_deauthenticated_projection(&self) {
        let mut projection = self.auth_projection.lock().await;
        if self.client.is_authenticated() {
            return;
        }
        projection.hydrated = false;
        projection.skip_next_authenticated_update = false;
        self.reset_account_projection();
    }

    fn reset_account_projection(&self) {
        reset_account_projection(&self.current, &self.subscription_projection);
    }

    fn reset_account_projection_for_authentication(&self) {
        self.reset_account_projection();
        self.current.update(json!({
            "networkStatus": if self.client.is_socket_connected() { "connected" } else { "disconnected" },
            "network": self.client.network().as_str(),
        }));
    }

    fn rollback_failed_initial_hydration(
        &self,
        projection: &mut AuthProjectionState,
        clear_installed_auth: bool,
    ) {
        if clear_installed_auth {
            self.client.clear_auth();
        }
        projection.hydrated = false;
        projection.skip_next_authenticated_update = false;
        self.reset_account_projection();
    }

    pub async fn account_balance(&self) -> Result<Value> {
        let response = self.client.rest.get("/v4/account/balance", None).await?;
        Ok(data(&response).clone())
    }

    pub async fn refresh_balance(&self) -> Result<Value> {
        let balance = self.account_balance().await?;
        self.current.update(json!({"balance": balance}));
        Ok(balance)
    }
}

pub(super) async fn fetch_current_account(client: &ApiClient) -> Result<Value> {
    let response = client.rest.get("/v1/account/me", None).await?;
    let payload = data(&response).clone();
    if !payload.is_object() {
        return Err(Error::Protocol(
            "account response did not include data".into(),
        ));
    }
    Ok(payload)
}

pub(super) fn apply_current_account(current: &CurrentAccount, payload: &Value) {
    current.update(json!({
        "username": payload.get("username"),
        "email": payload.get("currentEmail").or_else(|| payload.get("email")),
        "walletAddress": payload.get("walletAddress").or_else(|| payload.get("wallet_address")),
    }));
}

pub(super) fn reset_account_projection(
    current: &CurrentAccount,
    subscription: &Mutex<SubscriptionProjectionState>,
) {
    *subscription.lock() = SubscriptionProjectionState::default();
    current.clear();
}
