use std::time::Instant;

use anyhow::Result;
use sogni_client::{Network, ProjectRequest, ProjectsApi};

use crate::{
    common::{files::download, models::ImageModelSpec},
    config::Config,
    types::{CostModel, ModelResult, PhaseResult, RunRecord, TierSteps},
};

fn request(config: &Config, model: ImageModelSpec, steps: u32, seed: i64) -> ProjectRequest {
    let mut request = ProjectRequest::image(model.id, &config.prompt)
        .network(Network::from(config.network))
        .number_of_media(1)
        .dimensions(model.width, model.height)
        .steps(steps)
        .guidance(model.default_guidance)
        .param("seed", seed)
        .param("numberOfPreviews", 0)
        .param("disableNSFWFilter", false)
        .param("outputFormat", "jpg")
        .param("tokenType", config.token_type.as_str())
        .param("billingMode", config.billing_mode.as_str())
        .param("sampler", model.sampler)
        .param("scheduler", model.scheduler);
    if let Some(negative) = model.negative_prompt {
        request = request.param("negativePrompt", negative);
    }
    request
}

pub fn dry_run_requests(config: &Config) -> Vec<ProjectRequest> {
    config
        .models
        .iter()
        .flat_map(|model| {
            let tier = crate::config::tier(model.id).expect("every benchmark model has a tier");
            let mut steps = vec![tier.min];
            if tier.default != tier.min && tier.default != tier.max {
                steps.push(tier.default);
            }
            steps.push(tier.max);
            steps
                .into_iter()
                .map(|steps| request(config, *model, steps, 0))
        })
        .collect()
}

async fn single(
    projects: &ProjectsApi,
    config: &Config,
    model: ImageModelSpec,
    steps: u32,
) -> (u64, i64, Result<String>) {
    let seed = i64::from(rand::random::<u32>() & i32::MAX as u32);
    let started = Instant::now();
    let result = async {
        let project = projects.create(request(config, model, steps, seed)).await?;
        let urls = project.wait_for_completion(None).await?;
        urls.into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("project returned no result URL"))
    }
    .await;
    let millis = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    (millis, seed, result)
}

async fn phase(
    projects: &ProjectsApi,
    config: &Config,
    model: ImageModelSpec,
    steps: u32,
) -> PhaseResult {
    let measured_total = config.runs - config.warmup;
    let mut runs = Vec::with_capacity(config.runs);
    for run in 1..=config.runs {
        let is_warmup = run <= config.warmup;
        let label = if is_warmup {
            format!("Warmup {run}/{}", config.warmup)
        } else {
            format!("Run {}/{}", run - config.warmup, measured_total)
        };
        print!("    {label}: generating...");
        let (duration_ms, seed, result) = single(projects, config, model, steps).await;
        let (success, error) = match result {
            Ok(url) => {
                println!(
                    "\r    {label}: {}{suffix}",
                    duration(duration_ms),
                    suffix = if is_warmup {
                        " (warmup - discarded)"
                    } else {
                        ""
                    }
                );
                if config.download {
                    let filename = format!("{}_{}steps_run{run}_{seed}.jpg", model.key, steps);
                    if let Err(error) = download(&url, config.output.join(filename)).await {
                        eprintln!("      Warning: download failed: {error}");
                    }
                }
                (true, None)
            }
            Err(error) => {
                println!("\r    {label}: FAILED - {error}");
                (false, Some(error.to_string()))
            }
        };
        runs.push(RunRecord {
            run,
            is_warmup,
            duration_ms,
            success,
            error,
        });
    }
    summarize(steps, runs)
}

fn summarize(steps: u32, runs: Vec<RunRecord>) -> PhaseResult {
    // Only successful measured runs describe steady-state inference performance.
    let mut measured = runs
        .iter()
        .filter(|run| !run.is_warmup && run.success)
        .map(|run| run.duration_ms)
        .collect::<Vec<_>>();
    measured.sort_unstable();
    let avg_ms = (!measured.is_empty())
        .then(|| measured.iter().map(|value| *value as f64).sum::<f64>() / measured.len() as f64);
    let median_ms = (!measured.is_empty()).then(|| {
        let middle = measured.len() / 2;
        if measured.len() % 2 == 0 {
            (measured[middle - 1] as f64 + measured[middle] as f64) / 2.0
        } else {
            measured[middle] as f64
        }
    });
    PhaseResult {
        steps,
        measured_count: measured.len(),
        avg_ms,
        median_ms,
        min_ms: measured.first().copied(),
        max_ms: measured.last().copied(),
        warmup_ms: runs
            .iter()
            .find(|run| run.is_warmup && run.success)
            .map(|run| run.duration_ms),
        failed_count: runs.iter().filter(|run| !run.success).count(),
        runs,
    }
}

pub async fn model(
    projects: &ProjectsApi,
    config: &Config,
    model: ImageModelSpec,
    tier: TierSteps,
) -> ModelResult {
    println!(
        "\n{} ({}, {}x{})",
        model.name, model.id, model.width, model.height
    );
    println!("  Phase: {} steps (min)", tier.min);
    let minimum = phase(projects, config, model, tier.min).await;
    let default_matches = tier.default == tier.min || tier.default == tier.max;
    let explicit_default = if default_matches {
        None
    } else {
        println!("  Phase: {} steps (default validation)", tier.default);
        Some(phase(projects, config, model, tier.default).await)
    };
    println!("  Phase: {} steps (max)", tier.max);
    let maximum = phase(projects, config, model, tier.max).await;
    let default = explicit_default.unwrap_or_else(|| {
        if tier.default == tier.min {
            minimum.clone()
        } else {
            maximum.clone()
        }
    });
    let cost_model = derive_cost_model(tier, &minimum, &default, &maximum);
    ModelResult {
        model_key: model.key.into(),
        model_name: model.name.into(),
        model_id: model.id.into(),
        resolution: format!("{}x{}", model.width, model.height),
        tier_steps: tier,
        min_steps_benchmark: minimum,
        default_steps_benchmark: default,
        default_matches_min_or_max: default_matches,
        max_steps_benchmark: maximum,
        cost_model,
    }
}

fn derive_cost_model(
    tier: TierSteps,
    min: &PhaseResult,
    default: &PhaseResult,
    max: &PhaseResult,
) -> CostModel {
    // The endpoints define a simple explanatory fit; the default tier validates it.
    let Some((minimum, maximum)) = min.avg_ms.zip(max.avg_ms) else {
        return CostModel {
            base_ms: None,
            per_step_ms: None,
            default_derived_ms: None,
            default_measured_ms: default.avg_ms.map(|v| v.round() as i64),
            default_error_ms: None,
            default_error_pct: None,
        };
    };
    let per_step = (maximum - minimum) / f64::from(tier.max - tier.min);
    let base = minimum - f64::from(tier.min) * per_step;
    let derived = base + f64::from(tier.default) * per_step;
    let measured = default.avg_ms;
    CostModel {
        base_ms: Some(base.round() as i64),
        per_step_ms: Some(per_step.round() as i64),
        default_derived_ms: Some(derived.round() as i64),
        default_measured_ms: measured.map(|value| value.round() as i64),
        default_error_ms: measured.map(|value| (value - derived).round() as i64),
        default_error_pct: measured.map(|value| ((value - derived) / derived) * 100.0),
    }
}

pub fn duration(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else {
        format!("{:.2}s", milliseconds as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_statistics_exclude_warmups_and_failures() {
        let runs = vec![
            RunRecord {
                run: 1,
                is_warmup: true,
                duration_ms: 9_000,
                success: true,
                error: None,
            },
            RunRecord {
                run: 2,
                is_warmup: false,
                duration_ms: 100,
                success: true,
                error: None,
            },
            RunRecord {
                run: 3,
                is_warmup: false,
                duration_ms: 300,
                success: true,
                error: None,
            },
            RunRecord {
                run: 4,
                is_warmup: false,
                duration_ms: 50,
                success: false,
                error: Some("failed".into()),
            },
        ];
        let result = summarize(4, runs);
        assert_eq!(result.avg_ms, Some(200.0));
        assert_eq!(result.median_ms, Some(200.0));
        assert_eq!(result.failed_count, 1);
    }
}
