#[cfg(feature = "wallet")]
use std::time::Duration;

use serde_json::{Value, json};

#[cfg(feature = "wallet")]
use super::wallet_crypto::{amount_to_wei, sign_dynamic};
use super::{AccountApi, data};
use crate::{Error, Network, Result};

#[cfg(feature = "wallet")]
const INSUFFICIENT_ALLOWANCE: i64 = 149;
#[cfg(feature = "wallet")]
const MAX_DEPOSIT_ATTEMPTS: usize = 4;

impl AccountApi {
    pub async fn wallet_balance(&self, wallet_address: &str, provider: &str) -> Result<Value> {
        let response = self
            .client
            .rest
            .get(
                "/v2/wallet/balance",
                Some(&json!({"walletAddress": wallet_address, "provider": provider})),
            )
            .await?;
        Ok(data(&response).clone())
    }

    pub async fn validate_username(&self, username: &str) -> Result<Value> {
        match self
            .client
            .rest
            .post(
                "/v1/account/username/validate",
                &json!({"username": username}),
            )
            .await
        {
            Err(Error::Api(error)) if error.error_code.as_i64() == Some(108) => Ok(error.payload),
            result => result,
        }
    }

    pub async fn switch_network(&self, network: Network) -> Result<Network> {
        self.current
            .update(json!({"networkStatus": "switching", "network": Value::Null}));
        let network = self.client.switch_network(network).await?;
        self.current
            .update(json!({"networkStatus": "connected", "network": network.as_str()}));
        Ok(network)
    }

    pub async fn transaction_history(&self, query: &Value) -> Result<Value> {
        let response = self
            .client
            .rest
            .get("/v1/transactions/list", Some(query))
            .await?;
        Ok(data(&response).clone())
    }

    pub async fn rewards(&self, query: Option<&Value>) -> Result<Vec<Value>> {
        let response = self.client.rest.get("/v4/account/rewards", query).await?;
        Ok(data(&response)
            .get("rewards")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn claim_rewards(
        &self,
        reward_ids: &[String],
        provider: &str,
        turnstile_token: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({"claims": reward_ids, "provider": provider});
        if let Some(token) = turnstile_token {
            body["turnstileToken"] = json!(token);
        }
        self.client
            .rest
            .post("/v3/account/reward/claim", &body)
            .await?;
        Ok(())
    }

    #[cfg(feature = "wallet")]
    pub async fn withdraw(&self, password: &str, amount: &str, provider: &str) -> Result<()> {
        use ethers_signers::Signer as _;

        let wallet = self.current_wallet(password)?;
        let address = format!("{:#x}", wallet.address());
        let payload = json!({
            "walletAddress": address,
            "amount": amount_to_wei(amount)?,
            "provider": provider,
        });
        let response = self
            .client
            .rest
            .post("/v1/account/token/withdraw/permit", &payload)
            .await?;
        let permit = data(&response);
        let signature = sign_dynamic(&wallet, permit).await?;
        let mut body = payload;
        body["signature"] = json!(signature);
        self.client
            .rest
            .post("/v2/account/token/withdraw", &body)
            .await?;
        Ok(())
    }

    #[cfg(feature = "wallet")]
    pub async fn deposit(&self, password: &str, amount: &str, provider: &str) -> Result<()> {
        use ethers_signers::Signer as _;

        let wallet = self.current_wallet(password)?;
        let address = format!("{:#x}", wallet.address());
        let body = json!({
            "walletAddress": address,
            "amount": amount_to_wei(amount)?,
            "provider": provider,
        });
        for attempt in 1..=MAX_DEPOSIT_ATTEMPTS {
            match self
                .client
                .rest
                .post("/v3/account/token/deposit", &body)
                .await
            {
                Ok(_) => return Ok(()),
                Err(Error::Api(error))
                    if error.error_code.as_i64() == Some(INSUFFICIENT_ALLOWANCE) =>
                {
                    if attempt == 1 {
                        self.approve_token_usage(password, "account", provider)
                            .await?;
                    }
                    if attempt == MAX_DEPOSIT_ATTEMPTS {
                        return Err(Error::Api(error));
                    }
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::Protocol(
            "deposit retry loop ended unexpectedly".into(),
        ))
    }

    #[cfg(feature = "wallet")]
    pub async fn approve_token_usage(
        &self,
        password: &str,
        spender: &str,
        provider: &str,
    ) -> Result<()> {
        use ethers_signers::Signer as _;

        if !matches!(spender, "account" | "staker") {
            return Err(Error::InvalidInput(
                "spender must be 'account' or 'staker'".into(),
            ));
        }
        let wallet = self.current_wallet(password)?;
        let address = format!("{:#x}", wallet.address());
        let response = self
            .client
            .rest
            .post(
                "/v1/contract/token/approve/permit",
                &json!({"walletAddress": address, "spender": spender, "provider": provider}),
            )
            .await?;
        let permit = data(&response);
        let signature = sign_dynamic(&wallet, permit).await?;
        let deadline = permit
            .get("message")
            .and_then(|message| message.get("deadline"))
            .cloned()
            .ok_or_else(|| Error::Protocol("approval permit has no message.deadline".into()))?;
        self.client
            .rest
            .post(
                "/v1/contract/token/approve",
                &json!({
                    "walletAddress": address,
                    "spender": spender,
                    "provider": provider,
                    "deadline": deadline,
                    "approveSignature": signature,
                }),
            )
            .await?;
        Ok(())
    }
}
