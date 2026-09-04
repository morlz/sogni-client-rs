use std::{path::PathBuf, time::Duration};

use crate::common::{
    auth::{close, connect},
    cli::{confirm_estimate, execution_requested, explain_dry_run},
    files::download_results,
    progress::wait_with_progress,
    workflow::{estimate_image, print_request},
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::Value;
use sogni_client::{
    ACTIVE_PROJECTS_RECOVERED_EVENT, COMPLETED_PROJECTS_RECOVERED_EVENT, Network, ProjectRequest,
    ProjectResolution, ProjectStatus, ResolveMissingOptions,
};

const PREFERRED: [&str; 4] = [
    "z_image_bf16",
    "chroma1-hd_fp8_scaled",
    "krea2_turbo_fp8_scaled",
    "flux1-schnell-fp8",
];

#[derive(Debug, Parser)]
#[command(about = "Demonstrate durable project recovery after a socket drop")]
struct Args {
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value_t = 3)]
    number: u32,
    #[arg(long, default_value_t = 120)]
    first_event_timeout: u64,
    #[arg(long, default_value_t = 240)]
    completion_timeout: u64,
    #[arg(long)]
    resolve_id: Vec<String>,
    #[arg(long, default_value = "output")]
    output: PathBuf,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn request(model: &str, number: u32) -> ProjectRequest {
    ProjectRequest::image(
        model,
        "a lighthouse on a cliff at dusk, painterly, warm light",
    )
    .network(Network::Fast)
    .number_of_media(number)
    .steps(28)
    .guidance(5.0)
    .dimensions(1024, 1024)
    .param("tokenType", "spark")
}
fn ids(value: &Value, path: &str) -> usize {
    value
        .pointer(path)
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    if args.number == 0 {
        bail!("--number must be positive");
    }
    if !execution_requested(args.execute, args.dry_run)? {
        let preview = request(args.model.as_deref().unwrap_or(PREFERRED[0]), args.number);
        print_request(&preview)?;
        println!(
            "Recovery events: {ACTIVE_PROJECTS_RECOVERED_EVENT}, {COMPLETED_PROJECTS_RECOVERED_EVENT}"
        );
        println!(
            "Plan: create with client A, close A after its first job event, reconnect client B with the same app id, sync, finish, and resolve missing ids."
        );
        explain_dry_run();
        return Ok(());
    }
    let app_id = format!("sogni-rust-resume-{}", std::process::id());
    let client_a = connect(app_id.clone(), Network::Fast).await?;
    let result_a = async {
        let models = client_a
            .projects
            .wait_for_models(Duration::from_secs(20))
            .await?;
        let model = select_model(args.model.as_deref(), &models)?;
        let request = request(&model, args.number);
        let estimate = estimate_image(&client_a.projects, &request).await?;
        confirm_estimate(&estimate, args.yes)?;
        let mut events = client_a.projects.subscribe();
        let project = client_a.projects.create(request).await?;
        let id = project.id();
        println!("Client A created {id}");
        tokio::time::timeout(Duration::from_secs(args.first_event_timeout), async {
            loop {
                let event = events
                    .recv()
                    .await
                    .context("project event channel closed")?;
                if event.name == "job"
                    && event.data.get("projectId").and_then(Value::as_str) == Some(id.as_str())
                {
                    break Ok::<(), anyhow::Error>(());
                }
            }
        })
        .await
        .context("timed out waiting for the first job event")??;
        let before = project.status();
        client_a.close().await?;
        println!(
            "Client A dropped its socket: {before:?} -> {:?}",
            project.status()
        );
        if project.status() == ProjectStatus::Failed {
            bail!("project failed merely because the socket disconnected");
        }
        Ok::<String, anyhow::Error>(id)
    }
    .await;
    let project_id = match result_a {
        Ok(id) => id,
        Err(error) => {
            let _ = client_a.close().await;
            return Err(error);
        }
    };
    let client_b = connect(app_id, Network::Fast).await?;
    let result_b = async {
        let sync = client_b.projects.sync("resume-example").await?;
        println!("Recovery sync: {}", serde_json::to_string_pretty(&sync)?);
        let tracked = client_b
            .projects
            .tracked_projects()
            .into_iter()
            .find(|project| project.id() == project_id)
            .ok_or_else(|| anyhow::anyhow!("client B did not rebuild project {project_id}"))?;
        if !tracked.snapshot().recovered {
            bail!("rehydrated project is not marked recovered");
        }
        let urls = tokio::time::timeout(
            Duration::from_secs(args.completion_timeout),
            wait_with_progress(&tracked),
        )
        .await
        .context("timed out waiting for recovered project")??;
        if urls.len() != args.number as usize || tracked.status() != ProjectStatus::Completed {
            bail!("recovered project completed incompletely");
        }
        download_results(&urls, &args.output, "recovered-image", "png").await?;
        let manual = client_b.projects.sync("resume-example-final").await?;
        println!(
            "Final sync: active={}, unclaimed={}, lost={}",
            ids(&manual, "/snapshot/activeProjects"),
            ids(&manual, "/snapshot/unclaimedCompletedProjects"),
            ids(&manual, "/lost")
        );
        if ids(&manual, "/snapshot/activeProjects") != 0
            || ids(&manual, "/snapshot/unclaimedCompletedProjects") != 0
            || ids(&manual, "/lost") != 0
        {
            bail!("final recovery snapshot was not clean");
        }
        let mut resolve = args.resolve_id.clone();
        resolve.push(project_id.clone());
        let resolutions = client_b
            .projects
            .resolve_missing(
                &resolve,
                Some(ResolveMissingOptions {
                    attempts: 3,
                    retry_delay: Duration::from_millis(500),
                }),
            )
            .await;
        for (id, state) in resolutions {
            println!("Resolution {id}: {}", state.state());
            if id == project_id && !matches!(state, ProjectResolution::Finished { .. }) {
                bail!("completed project did not resolve as finished");
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if result_b.is_err() {
        let _ = client_b.projects.cancel(&project_id).await;
    }
    let closed = close(&client_b).await;
    result_b.and(closed)
}

fn select_model(explicit: Option<&str>, models: &[Value]) -> Result<String> {
    let online = |id: &str| {
        models.iter().find(|model| {
            model.get("id").and_then(Value::as_str) == Some(id)
                && model
                    .get("workerCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    > 0
        })
    };
    if let Some(id) = explicit {
        return online(id)
            .map(|_| id.to_owned())
            .ok_or_else(|| anyhow::anyhow!("model {id} has no available workers"));
    }
    for id in PREFERRED {
        if online(id).is_some() {
            return Ok(id.into());
        }
    }
    models
        .iter()
        .find(|m| m.get("workerCount").and_then(Value::as_u64).unwrap_or(0) > 0)
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("no online image model is available"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dry_request_keeps_recovery_workload_shape() {
        let p = request("z_image_bf16", 3).params();
        assert_eq!(p["numberOfMedia"], 3);
        assert_eq!(p["network"], "fast");
    }
}
