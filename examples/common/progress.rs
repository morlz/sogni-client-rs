use std::{
    collections::HashSet,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::Result;
use sogni_client::{Project, ProjectStatus, ProjectsApi};
use tokio::task::JoinHandle;

use super::files::download;

/// Print project progress until the project reaches a terminal state.
pub fn spawn_project_reporter(project: Project) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut events = project.subscribe();
        while let Ok(event) = events.recv().await {
            let snapshot = project.snapshot();
            print!(
                "\rProject {}: {:>3}% ({:?})",
                snapshot.id, snapshot.progress, snapshot.status
            );
            let _ = io::stdout().flush();
            if snapshot.status.is_finished() {
                println!();
                break;
            }
            if event.name == "jobFailed" {
                println!("\nJob failure: {}", event.data);
            }
        }
    })
}

/// Print raw project/job API events. Useful when demonstrating event-driven use.
pub fn spawn_api_reporter(projects: ProjectsApi) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut events = projects.subscribe();
        while let Ok(event) = events.recv().await {
            if matches!(event.name.as_str(), "project" | "job" | "projectsSynced") {
                println!("{}: {}", event.name, event.data);
            }
        }
    })
}

pub async fn wait_with_progress(project: &Project) -> Result<Vec<String>> {
    let reporter = spawn_project_reporter(project.clone());
    let result = project.wait_for_completion(None).await;
    reporter.abort();
    if project.status() != ProjectStatus::Pending {
        println!(
            "Project {} finished as {:?}",
            project.id(),
            project.status()
        );
    }
    Ok(result?)
}

/// Poll job snapshots and save each distinct preview URL while a project runs.
pub fn spawn_preview_downloader(
    project: Project,
    output_dir: PathBuf,
    prefix: String,
    extension: String,
) -> JoinHandle<Vec<PathBuf>> {
    tokio::spawn(async move {
        let mut seen = HashSet::new();
        let mut saved = Vec::new();
        loop {
            for job in project.jobs() {
                let Some(url) = job.snapshot().preview_url else {
                    continue;
                };
                if !seen.insert(url.clone()) {
                    continue;
                }
                let name = format!("{prefix}-preview-{}.{}", seen.len(), extension);
                match download(&url, output_dir.join(name)).await {
                    Ok(path) => {
                        println!("Preview saved: {}", path.display());
                        saved.push(path);
                    }
                    Err(error) => eprintln!("Preview download failed: {error}"),
                }
            }
            if project.status().is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        saved
    })
}
