use std::{fmt, sync::Weak};

use serde::Deserialize;
use zeroize::Zeroizing;

use super::*;
use crate::{Error, Result, auth::RequestSession};

/// An incomplete sign-in. Keep it in memory and pass it back to the originating
/// client with a second factor. Its temporary credential is never printed.
#[derive(thiserror::Error)]
#[error("Enter your authenticator code or a backup code to finish signing in.")]
pub struct TwoFactorChallenge {
    token: Zeroizing<String>,
    client: Weak<ApiClient>,
    session: RequestSession,
    #[cfg(feature = "wallet")]
    verify: AsyncMutex<()>,
    wallet_address: String,
    username: String,
}

impl fmt::Debug for TwoFactorChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TwoFactorChallenge").finish_non_exhaustive()
    }
}

impl TwoFactorChallenge {
    #[cfg(feature = "wallet")]
    pub(in crate::account) fn new(
        client: &Arc<ApiClient>,
        session: RequestSession,
        wallet_address: String,
        username: &str,
        response: &Value,
    ) -> Result<Self> {
        let token = response
            .get("tempToken")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                Error::Protocol("two-factor challenge is missing its temporary credential".into())
            })?;
        Ok(Self {
            token: Zeroizing::new(token.to_owned()),
            client: Arc::downgrade(client),
            session,
            verify: AsyncMutex::new(()),
            wallet_address,
            username: response
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or(username)
                .to_owned(),
        })
    }

    fn check(&self, client: &Arc<ApiClient>) -> Result<()> {
        if !self.client.ptr_eq(&Arc::downgrade(client)) {
            return Err(Error::InvalidInput(
                "two-factor challenge belongs to a different client".into(),
            ));
        }
        self.session.check()
    }

    #[must_use]
    pub fn wallet_address(&self) -> &str {
        &self.wallet_address
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }
}

/// A native authenticator or backup code, or a WebAuthn assertion provided by
/// the application. The SDK does not run a browser or collect credentials.
pub enum TwoFactorProof {
    Code(String),
    Passkey {
        assertion: Value,
        challenge_id: String,
    },
}

impl fmt::Debug for TwoFactorProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TwoFactorProof").finish_non_exhaustive()
    }
}

#[cfg(feature = "wallet")]
impl TwoFactorProof {
    fn fields(&self) -> serde_json::Map<String, Value> {
        match self {
            Self::Code(code) => [("code".to_owned(), json!(code.trim().to_uppercase()))]
                .into_iter()
                .collect(),
            Self::Passkey {
                assertion,
                challenge_id,
            } => [
                ("assertion".to_owned(), assertion.clone()),
                ("challengeId".to_owned(), json!(challenge_id)),
            ]
            .into_iter()
            .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorStatus {
    pub enabled: bool,
    pub backup_codes_remaining: u32,
    pub can_manage: bool,
    pub authenticator_enabled: bool,
    pub passkeys: Vec<TwoFactorPasskey>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TwoFactorPasskey {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyAuthenticationOptions {
    pub challenge_id: String,
    pub options: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyRegistrationOptions {
    pub setup_id: String,
    pub options: Value,
    pub backup_codes: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorSetup {
    pub setup_id: String,
    pub secret: String,
    pub qr_data: String,
    pub backup_codes: Vec<String>,
}

impl AccountApi {
    /// Get the available second factors for the signed-in account.
    pub async fn get_two_factor_status(&self) -> Result<TwoFactorStatus> {
        let response = self.client.rest.get("/v1/account/2fa/status", None).await?;
        Ok(TwoFactorStatus::deserialize(data(&response))?)
    }

    /// Fetch WebAuthn options for an incomplete login, or for the current account.
    pub async fn get_two_factor_passkey_options(
        &self,
        challenge: Option<&TwoFactorChallenge>,
    ) -> Result<PasskeyAuthenticationOptions> {
        let response = match challenge {
            Some(challenge) => {
                challenge.check(&self.client)?;
                challenge
                    .session
                    .run(self.client.rest.post_sign_in(
                        "/v1/account/2fa/passkeys/login-options",
                        &json!({"tempToken":challenge.token.as_str()}),
                        false,
                    ))
                    .await?
            }
            None => {
                self.client
                    .rest
                    .post(
                        "/v1/account/2fa/passkeys/authentication-options",
                        &json!({}),
                    )
                    .await?
            }
        };
        Ok(PasskeyAuthenticationOptions::deserialize(data(&response))?)
    }

    /// Complete the challenge and initialize the account and socket as for login.
    #[cfg(feature = "wallet")]
    pub async fn verify_two_factor_login(
        &self,
        challenge: &TwoFactorChallenge,
        proof: &TwoFactorProof,
    ) -> Result<Value> {
        challenge.check(&self.client)?;
        let session = &challenge.session;
        let _verify = session
            .run(async { Ok(challenge.verify.lock().await) })
            .await?;
        let mut body = proof.fields();
        body.insert("tempToken".into(), json!(challenge.token.as_str()));
        let response = session
            .run(self.client.rest.post_sign_in(
                "/v1/account/2fa/verify-login",
                &Value::Object(body),
                true,
            ))
            .await?;
        let mut payload = data(&response)
            .as_object()
            .cloned()
            .ok_or_else(|| Error::Protocol("invalid two-factor sign-in response".into()))?;
        if payload.get("requires2FA").and_then(Value::as_bool) == Some(true) {
            return Err(Error::Protocol(
                "two-factor sign-in did not complete".into(),
            ));
        }
        payload.remove("tempToken");
        payload.insert("username".into(), json!(challenge.username));
        let payload = Value::Object(payload);
        self.authenticate_response_and_hydrate(&payload, &challenge.wallet_address, session)
            .await?;
        self.client.start().await?;
        Ok(payload)
    }

    pub async fn finish_passkey_setup(&self, setup_id: &str, response: &Value) -> Result<()> {
        self.client
            .rest
            .post(
                "/v1/account/2fa/passkeys/register",
                &json!({"setupId":setup_id,"response":response}),
            )
            .await?;
        Ok(())
    }

    pub async fn confirm_two_factor_setup(&self, setup_id: &str, code: &str) -> Result<()> {
        self.client
            .rest
            .post(
                "/v1/account/2fa/verify-setup",
                &json!({"setupId":setup_id,"code":code.trim()}),
            )
            .await?;
        Ok(())
    }
}

#[cfg(feature = "wallet")]
mod management;
#[cfg(all(test, feature = "wallet"))]
mod tests;
