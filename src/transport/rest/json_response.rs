use super::*;

impl RestClient {
    /// Do not stage credentials from the first half of a two-factor sign-in.
    pub(crate) async fn post_sign_in(
        &self,
        path: &str,
        body: &Value,
        accept_cookies: bool,
    ) -> Result<Value> {
        let url = self.url(path)?;
        let request = self
            .authenticated_http
            .post(url.clone())
            .json(body)
            .timeout(self.timeout);
        let (response, generation) = self
            .send_with_cookie_generation(request, &url, None, path == "/v1/account/login")
            .await?;
        let headers = response.headers().clone();
        let response_url = response.url().clone();
        let session = response.extensions().get::<RequestSession>().cloned();
        let value = self.process_response(response).await?;
        if let Some(session) = session {
            session.check()?;
        }
        let data = value.get("data").unwrap_or(&value);
        if accept_cookies
            && data.is_object()
            && data.get("requires2FA").and_then(Value::as_bool) != Some(true)
        {
            self.cookies
                .store_response(generation, &headers, &response_url);
        }
        Ok(value)
    }

    pub async fn process_response(&self, response: reqwest::Response) -> Result<Value> {
        let session = response
            .extensions()
            .get::<RequestSession>()
            .cloned()
            .unwrap_or_else(|| self.request_session());
        Self::read_json_response(response, Some(&session)).await
    }

    pub(crate) async fn get_public(&self, path: &str, query: Option<&Value>) -> Result<Value> {
        if self.request_session().is_closed() {
            return Err(Error::Closed);
        }
        let mut request = self.authenticated_http.get(self.url(path)?);
        if let Some(query) = query {
            request = request.query(&query_pairs(query));
        }
        Self::read_json_response(request.timeout(self.timeout).send().await?, None).await
    }

    async fn read_json_response(
        response: reqwest::Response,
        session: Option<&RequestSession>,
    ) -> Result<Value> {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let text = match session {
            Some(session) => session.run(async { Ok(response.text().await?) }).await?,
            None => response.text().await?,
        };
        let parsed = if text.trim().is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(&text).ok()
        };
        if !status.is_success() {
            let mut payload = parsed.filter(Value::is_object).unwrap_or_else(|| {
                let message = non_json_error_message(status, &text);
                json!({"status": "error", "message": message, "errorCode": status.as_u16()})
            });
            crate::media_encryption::redact_media_keys(&mut payload);
            return Err(ApiError::new(status.as_u16(), payload)
                .with_retry_after(retry_after.as_deref())
                .into());
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        parsed.ok_or_else(|| {
            Error::Protocol(format!(
                "failed to parse JSON response body (HTTP {})",
                status.as_u16()
            ))
        })
    }
}
