use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde_json::json;

use crate::{
    config::Config,
    run::duration,
    types::{ModelResult, PhaseResult},
};

pub fn print_configuration(config: &Config) -> usize {
    let phases = config
        .models
        .iter()
        .filter_map(|model| crate::config::tier(model.id))
        .map(|tier| {
            if tier.default == tier.min || tier.default == tier.max {
                2
            } else {
                3
            }
        })
        .sum::<usize>();
    let generations = phases * config.runs;
    println!("Benchmark configuration:");
    println!("  Network:             {}", config.network.as_str());
    println!("  Models:              {}", config.models.len());
    println!("  Runs per step count: {}", config.runs);
    println!("  Warmup runs:         {}", config.warmup);
    println!("  Total generations:   {generations}");
    println!("  Download images:     {}", config.download);
    println!("  Output:              {}", config.output.display());
    println!("  Billing:             {}", config.billing_mode.as_str());
    generations
}

pub fn print_results(results: &[ModelResult]) {
    println!("\nBENCHMARK RESULTS SUMMARY");
    println!(
        "{:<27}{:<8}{:<11}{:<11}{:<11}{:<11}{:<7}",
        "Model", "Steps", "Avg", "Median", "Min", "Max", "Fails"
    );
    println!("{}", "-".repeat(86));
    for result in results {
        let mut phases = vec![&result.min_steps_benchmark];
        if !result.default_matches_min_or_max {
            phases.push(&result.default_steps_benchmark);
        }
        phases.push(&result.max_steps_benchmark);
        for phase in phases {
            print_phase(&result.model_name, phase);
        }
    }
    println!("\nDerived model: time = base + steps * per-step");
    let mut ranked = results
        .iter()
        .filter(|result| result.cost_model.per_step_ms.is_some())
        .collect::<Vec<_>>();
    ranked.sort_by_key(|result| result.cost_model.per_step_ms);
    for (index, result) in ranked.iter().enumerate() {
        println!(
            "  {:>2}. {:<28} {}/step (base {})",
            index + 1,
            result.model_name,
            signed_duration(result.cost_model.per_step_ms),
            signed_duration(result.cost_model.base_ms)
        );
    }
}

fn print_phase(name: &str, phase: &PhaseResult) {
    let show = |value: Option<f64>| {
        value
            .map(|value| duration(value.round().max(0.0) as u64))
            .unwrap_or_else(|| "N/A".into())
    };
    println!(
        "{:<27}{:<8}{:<11}{:<11}{:<11}{:<11}{:<7}",
        name.chars().take(25).collect::<String>(),
        phase.steps,
        show(phase.avg_ms),
        show(phase.median_ms),
        phase.min_ms.map(duration).unwrap_or_else(|| "N/A".into()),
        phase.max_ms.map(duration).unwrap_or_else(|| "N/A".into()),
        phase.failed_count
    );
}

fn signed_duration(value: Option<i64>) -> String {
    value
        .map(|value| {
            let sign = if value < 0 { "-" } else { "" };
            format!("{sign}{}", duration(value.unsigned_abs()))
        })
        .unwrap_or_else(|| "N/A".into())
}

pub fn save(config: &Config, results: &[ModelResult]) -> Result<()> {
    fs::create_dir_all(&config.output)
        .with_context(|| format!("create {}", config.output.display()))?;
    let output = json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "config": {
            "network": config.network.as_str(),
            "prompt": config.prompt,
            "runsPerStepCount": config.runs,
            "warmupRuns": config.warmup,
            "measuredRuns": config.runs - config.warmup,
            "tokenType": config.token_type.as_str(),
            "billingMode": config.billing_mode.as_str(),
        },
        "results": results,
    });
    let path = Path::new(&config.output).join("benchmark_results.json");
    fs::write(&path, serde_json::to_vec_pretty(&output)?)
        .with_context(|| format!("write {}", path.display()))?;
    println!("Results saved to {}", path.display());
    Ok(())
}
