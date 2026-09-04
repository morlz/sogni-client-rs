use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sogni_client::{CostEstimate, Network, ProjectRequest, ProjectsApi};
use tokio::sync::Semaphore;

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{execution_requested, explain_dry_run},
    workflow::{estimate_image, print_request, require_model},
};

#[derive(Debug, Parser)]
#[command(about = "Serve the Sogni image demo with Axum")]
pub struct Args {
    #[arg(long, default_value = "127.0.0.1:3000")]
    listen: SocketAddr,
    #[arg(long, default_value = "flux1-schnell-fp8")]
    model: String,
    #[arg(long, default_value_t = 4)]
    steps: u32,
    #[arg(long, default_value_t = 2)]
    max_concurrent: usize,
    #[arg(long, default_value_t = 600)]
    timeout_seconds: u64,
    /// Permit binding to a non-loopback interface.
    #[arg(long)]
    allow_remote: bool,
    /// Start the server and connect to Sogni.
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone)]
struct AppState {
    projects: ProjectsApi,
    model: Arc<str>,
    steps: u32,
    timeout: Duration,
    permits: Arc<Semaphore>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GenerateInput {
    prompt: String,
    #[serde(default)]
    style: String,
    #[serde(default)]
    confirmed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EstimateOutput {
    estimate: CostEstimate,
}

#[derive(Serialize)]
struct GenerateOutput {
    url: String,
}

#[derive(Serialize)]
struct ErrorOutput {
    error: &'static str,
}

struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(ErrorOutput { error: self.1 })).into_response()
    }
}

fn project_request(state: &AppState, input: &GenerateInput) -> Result<ProjectRequest, ApiError> {
    validate_input(input)?;
    let prompt = input.prompt.trim();
    let style = input.style.trim();

    Ok(ProjectRequest::image(state.model.as_ref(), prompt)
        .network(Network::Fast)
        .number_of_media(1)
        .steps(state.steps)
        .guidance(1.0)
        .param("stylePrompt", style)
        .param("tokenType", "spark")
        .param("outputFormat", "jpg"))
}

fn validate_input(input: &GenerateInput) -> Result<(), ApiError> {
    let prompt = input.prompt.trim();
    let style = input.style.trim();
    if prompt.is_empty() || prompt.chars().count() > 2_000 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "prompt must contain 1-2000 characters",
        ));
    }
    if style.chars().count() > 500 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "style must contain at most 500 characters",
        ));
    }
    Ok(())
}

async fn index() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' https: data:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; connect-src 'self'",
        ),
    );
    (headers, Html(include_str!("index.html")))
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn estimate(
    State(state): State<AppState>,
    Json(input): Json<GenerateInput>,
) -> Result<Json<EstimateOutput>, ApiError> {
    let request = project_request(&state, &input)?;
    let estimate = estimate_image(&state.projects, &request)
        .await
        .map_err(|_| ApiError(StatusCode::BAD_GATEWAY, "cost estimate failed"))?;
    Ok(Json(EstimateOutput { estimate }))
}

async fn generate(
    State(state): State<AppState>,
    Json(input): Json<GenerateInput>,
) -> Result<Json<GenerateOutput>, ApiError> {
    if !input.confirmed {
        return Err(ApiError(
            StatusCode::PRECONDITION_REQUIRED,
            "confirm the current estimate before generation",
        ));
    }
    let request = project_request(&state, &input)?;
    let _permit = state
        .permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError(StatusCode::TOO_MANY_REQUESTS, "generation capacity is busy"))?;
    let project = state
        .projects
        .create(request)
        .await
        .map_err(|_| ApiError(StatusCode::BAD_GATEWAY, "generation submission failed"))?;
    let urls = project
        .wait_for_completion(Some(state.timeout))
        .await
        .map_err(|_| ApiError(StatusCode::BAD_GATEWAY, "generation failed"))?;
    let url = urls.into_iter().next().ok_or(ApiError(
        StatusCode::BAD_GATEWAY,
        "generation returned no image",
    ))?;
    Ok(Json(GenerateOutput { url }))
}

pub async fn run(args: Args) -> Result<()> {
    if args.max_concurrent == 0 {
        bail!("--max-concurrent must be at least 1");
    }
    if args.timeout_seconds == 0 {
        bail!("--timeout-seconds must be at least 1");
    }
    if !args.listen.ip().is_loopback() && !args.allow_remote {
        bail!(
            "non-loopback binding requires --allow-remote; add authentication before public exposure"
        );
    }
    let sample = ProjectRequest::image(&args.model, "A paper-cut forest at sunrise")
        .network(Network::Fast)
        .steps(args.steps)
        .guidance(1.0)
        .number_of_media(1)
        .param("stylePrompt", "storybook")
        .param("tokenType", "spark");
    println!("HTTP demo listen address: http://{}", args.listen);
    print_request(&sample)?;
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-axum-demo"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, &args.model).await?;
        let state = AppState {
            projects: client.projects.clone(),
            model: Arc::from(args.model),
            steps: args.steps,
            timeout: Duration::from_secs(args.timeout_seconds),
            permits: Arc::new(Semaphore::new(args.max_concurrent)),
        };
        let app = Router::new()
            .route("/", get(index))
            .route("/healthz", get(health))
            .route("/api/estimate", post(estimate))
            .route("/api/generate", post(generate))
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(args.listen).await?;
        println!("Listening on http://{}; press Ctrl+C to stop", args.listen);
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_oversized_inputs() {
        assert!(
            validate_input(&GenerateInput {
                prompt: " ".into(),
                style: String::new(),
                confirmed: false
            })
            .is_err()
        );
        assert!(
            validate_input(&GenerateInput {
                prompt: "ok".into(),
                style: "x".repeat(501),
                confirmed: false
            })
            .is_err()
        );
    }
}
