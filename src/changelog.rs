use std::sync::Arc;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{ApiError, Error, Result, transport::ApiClient, utils::path_segment};

/// Public changelog. Reads do not require sign-in. Responses may be cached
/// for five minutes; poll no more often than that.
#[derive(Clone)]
pub struct ChangelogApi {
    client: Arc<ApiClient>,
}

impl std::fmt::Debug for ChangelogApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangelogApi").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ChangelogAudience {
    Artist,
    Developer,
    Worker,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ChangelogEntryType {
    New,
    Improved,
    Fixed,
    Release,
    Deprecated,
    Removed,
    Notice,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChangelogLink {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogEntry {
    pub id: String,
    pub url: String,
    pub title: String,
    pub summary: String,
    pub body_markdown: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: ChangelogEntryType,
    pub audiences: Vec<ChangelogAudience>,
    pub products: Vec<String>,
    pub version: Option<String>,
    pub links: Vec<ChangelogLink>,
    pub action_required: bool,
    pub effective_at: Option<String>,
    pub published_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChangelogProduct {
    pub id: String,
    pub label: String,
    pub audiences: Vec<ChangelogAudience>,
}

/// Each filter matches any of its values; the entry must match all supplied
/// filters. Dates are ISO 8601 and product IDs come from [ChangelogApi::products].
#[derive(Clone, Debug, Default)]
pub struct ListChangelogOptions {
    pub audience: Vec<ChangelogAudience>,
    pub product: Vec<String>,
    pub entry_type: Vec<ChangelogEntryType>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub action_required: bool,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListChangelogResult {
    pub entries: Vec<ChangelogEntry>,
    pub next_cursor: Option<String>,
}

impl ChangelogApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }

    /// One page of published entries, newest first.
    pub async fn list(&self, options: &ListChangelogOptions) -> Result<ListChangelogResult> {
        let query = list_query(options);
        let response = self
            .client
            .rest
            .get_public("/v1/changelog", Some(&query))
            .await?;
        parse_field(&response, "/data")
    }

    /// Read one published entry. A missing entry has status 404 and code 204.
    pub async fn get(&self, id: &str) -> Result<ChangelogEntry> {
        if matches!(id, "." | "..") {
            return Err(entry_not_found());
        }
        let response = self
            .client
            .rest
            .get_public(&format!("/v1/changelog/{}", path_segment(id)), None)
            .await?;
        if response.pointer("/data/entry").is_none_or(Value::is_null) {
            return Err(entry_not_found());
        }
        parse_field(&response, "/data/entry")
    }

    /// Products that can be used in the list filter, in display order.
    pub async fn products(&self) -> Result<Vec<ChangelogProduct>> {
        let response = self
            .client
            .rest
            .get_public("/v1/changelog/products", None)
            .await?;
        parse_field(&response, "/data/products")
    }
}

fn entry_not_found() -> Error {
    ApiError::new(
        404,
        json!({
            "status":"error","errorCode":204,"message":"Changelog entry not found"
        }),
    )
    .into()
}

fn parse_field<T: DeserializeOwned>(response: &Value, path: &str) -> Result<T> {
    let value = response
        .pointer(path)
        .ok_or_else(|| Error::Protocol(format!("Changelog response is missing {path}")))?;
    T::deserialize(value).map_err(Into::into)
}

fn list_query(options: &ListChangelogOptions) -> Value {
    fn csv(values: Value) -> Option<String> {
        let items: Vec<_> = values
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        (!items.is_empty()).then(|| items.join(","))
    }
    json!({
        "audience": csv(json!(options.audience)), "product": csv(json!(options.product)),
        "type": csv(json!(options.entry_type)), "since": options.since, "until": options.until,
        "actionRequired": options.action_required.then_some("true"),
        "limit": options.limit, "cursor": options.cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_documented_filter_names_and_csv_values() {
        let options = ListChangelogOptions {
            audience: vec![ChangelogAudience::Artist, ChangelogAudience::Developer],
            entry_type: vec![ChangelogEntryType::Removed],
            action_required: true,
            cursor: Some("next".into()),
            ..Default::default()
        };
        assert_eq!(
            crate::utils::query_pairs(&list_query(&options))
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
            std::collections::BTreeMap::from([
                ("actionRequired".to_owned(), "true".to_owned()),
                ("audience".to_owned(), "artist,developer".to_owned()),
                ("cursor".to_owned(), "next".to_owned()),
                ("type".to_owned(), "removed".to_owned()),
            ])
        );
        assert!(
            crate::utils::query_pairs(&list_query(&ListChangelogOptions::default())).is_empty()
        );
    }

    #[tokio::test]
    async fn public_changelog_does_not_send_or_invalidate_account_credentials() {
        use crate::SogniClient;
        use axum::{
            Json, Router,
            http::{HeaderMap, StatusCode},
            routing::get,
        };
        let app = Router::new()
            .route("/v1/account/me", get(|| async { Json(json!({"data":{"username":"fixture"}})) }))
            .route("/v1/changelog/products", get(|headers: HeaderMap| async move {
                assert!(!headers.contains_key("authorization"));
                    assert!(!headers.contains_key("api-key"));
                assert!(!headers.contains_key("cookie"));
                (StatusCode::UNAUTHORIZED, Json(json!({"status":"error","message":"public endpoint refused","errorCode":123})))
            }))
            .route("/v1/changelog/feed.json", get(|| async { Json(json!({"data":{"items":[]}})) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = SogniClient::builder()
            .disable_socket(true)
            .api_key("fixture")
            .rest_endpoint(endpoint)
            .build()
            .await
            .unwrap();
        assert!(
            matches!(client.changelog.products().await, Err(Error::Api(error)) if error.status == 401 && error.error_code == 123)
        );
        assert!(client.is_authenticated());
        assert!(
            matches!(client.changelog.get("feed.json").await, Err(Error::Api(error)) if error.status == 404 && error.error_code == 204)
        );
        client.close().await.unwrap();
        server.abort();
    }
}
