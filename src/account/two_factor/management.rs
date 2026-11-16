use super::*;
use crate::account::wallet_crypto::wallet;

impl AccountApi {
    pub async fn begin_passkey_setup(
        &self,
        username: &str,
        password: &str,
        name: &str,
        proof: Option<&TwoFactorProof>,
    ) -> Result<PasskeyRegistrationOptions> {
        let response = self
            .manage_factor(
                username,
                password,
                "passkeys/registration-options",
                json!({"name":name,"proof":proof.map(TwoFactorProof::fields)}),
            )
            .await?;
        Ok(PasskeyRegistrationOptions::deserialize(&response)?)
    }

    pub async fn remove_passkey(
        &self,
        username: &str,
        password: &str,
        credential_id: &str,
        proof: &TwoFactorProof,
    ) -> Result<()> {
        let mut body = proof.fields();
        body.insert("credentialId".into(), json!(credential_id));
        self.manage_factor(username, password, "passkeys/remove", Value::Object(body))
            .await?;
        Ok(())
    }

    pub async fn setup_two_factor(
        &self,
        username: &str,
        password: &str,
        proof: Option<&TwoFactorProof>,
    ) -> Result<TwoFactorSetup> {
        let response = self
            .manage_factor(
                username,
                password,
                "setup",
                json!({"proof":proof.map(TwoFactorProof::fields)}),
            )
            .await?;
        Ok(TwoFactorSetup::deserialize(&response)?)
    }

    pub async fn disable_two_factor(
        &self,
        username: &str,
        password: &str,
        proof: &TwoFactorProof,
    ) -> Result<()> {
        self.manage_factor(username, password, "disable", Value::Object(proof.fields()))
            .await?;
        Ok(())
    }

    pub async fn regenerate_two_factor_backup_codes(
        &self,
        username: &str,
        password: &str,
        proof: &TwoFactorProof,
    ) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BackupCodes {
            backup_codes: Vec<String>,
        }
        let response = self
            .manage_factor(
                username,
                password,
                "backup-codes",
                Value::Object(proof.fields()),
            )
            .await?;
        Ok(BackupCodes::deserialize(&response)?.backup_codes)
    }

    async fn manage_factor(
        &self,
        username: &str,
        password: &str,
        endpoint: &str,
        mut body: Value,
    ) -> Result<Value> {
        let session = self.client.rest.request_session();
        session
            .run(async {
                let wallet = wallet(username, password)?;
                let address = format!("{:#x}", wallet.address());
                if !self
                    .current_account()
                    .wallet_address()
                    .is_some_and(|current| current.eq_ignore_ascii_case(&address))
                {
                    return Err(Error::InvalidInput(
                        "Incorrect password. Please try again.".into(),
                    ));
                }
                let nonce = self.get_nonce(&address).await?;
                let signature = self.sign_named(
                &wallet, "Authentication",
                json!([{"name":"walletAddress","type":"address"},{"name":"nonce","type":"string"}]),
                json!({"walletAddress":address,"nonce":nonce}),
            ).await?;
                session.check()?;
                body["signature"] = json!(signature);
                let response = self
                    .client
                    .rest
                    .post(&format!("/v1/account/2fa/{endpoint}"), &body)
                    .await?;
                Ok(data(&response).clone())
            })
            .await
    }
}
