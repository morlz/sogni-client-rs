use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde_json::Value;
use sogni_client::{CostEstimate, Network};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum NetworkArg {
    #[default]
    Fast,
    Relaxed,
}

impl From<NetworkArg> for Network {
    fn from(value: NetworkArg) -> Self {
        match value {
            NetworkArg::Fast => Self::Fast,
            NetworkArg::Relaxed => Self::Relaxed,
        }
    }
}

impl NetworkArg {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Relaxed => "relaxed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum BillingMode {
    #[default]
    Auto,
    Subscription,
    Tokens,
}

impl BillingMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Subscription => "subscription",
            Self::Tokens => "tokens",
        }
    }

    pub fn from_environment() -> Result<Option<Self>> {
        let Some(value) = super::auth::setting("SOGNI_BILLING_MODE") else {
            return Ok(None);
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Some(Self::Auto)),
            "subscription" => Ok(Some(Self::Subscription)),
            "tokens" => Ok(Some(Self::Tokens)),
            _ => bail!("SOGNI_BILLING_MODE must be auto, subscription, or tokens"),
        }
    }
}

/// Resolve billing from an explicit CLI value, then `SOGNI_BILLING_MODE`, then `auto`.
pub fn resolve_billing_mode(explicit: Option<BillingMode>) -> Result<BillingMode> {
    Ok(explicit
        .or(BillingMode::from_environment()?)
        .unwrap_or_default())
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum TokenType {
    #[default]
    Spark,
    Sogni,
}

impl TokenType {
    pub fn from_environment() -> Result<Option<Self>> {
        let Some(value) = super::auth::setting("SOGNI_TOKEN_TYPE") else {
            return Ok(None);
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "spark" => Ok(Some(Self::Spark)),
            "sogni" => Ok(Some(Self::Sogni)),
            _ => bail!("SOGNI_TOKEN_TYPE must be spark or sogni"),
        }
    }
}

impl TokenType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spark => "spark",
            Self::Sogni => "sogni",
        }
    }
}

/// Resolve the standard execution guard used by every paid example.
///
/// The default is a credential-free dry run. A caller must pass `--execute`
/// before this returns `true`.
pub fn execution_requested(execute: bool, dry_run: bool) -> Result<bool> {
    if execute && dry_run {
        bail!("--execute and --dry-run cannot be used together");
    }
    Ok(execute)
}

pub fn explain_dry_run() {
    println!("Dry run only: no credentials were loaded and no network request was sent.");
    println!("Review the request above, then pass --execute to perform paid generation.");
}

pub fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    if !io::stdin().is_terminal() {
        bail!("interactive input is unavailable; provide the corresponding CLI option");
    }
    match default {
        Some(value) => print!("{label} [{value}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim();
    Ok(if value.is_empty() {
        default.unwrap_or_default().to_owned()
    } else {
        value.to_owned()
    })
}

pub fn confirm(label: &str, default: bool) -> Result<bool> {
    if !io::stdin().is_terminal() {
        bail!("confirmation requires a terminal; pass --yes for unattended execution");
    }
    let answer = prompt(label, Some(if default { "Y/n" } else { "y/N" }))?;
    if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
        Ok(true)
    } else if answer.eq_ignore_ascii_case("n") || answer.eq_ignore_ascii_case("no") {
        Ok(false)
    } else {
        Ok(default)
    }
}

pub fn require_confirmation(label: &str, assume_yes: bool) -> Result<()> {
    if assume_yes || confirm(label, false)? {
        Ok(())
    } else {
        bail!("operation cancelled")
    }
}

/// Print every available currency estimate and require opt-in before paid work.
pub fn confirm_estimate(estimate: &CostEstimate, assume_yes: bool) -> Result<()> {
    println!("Estimated cost:");
    println!("  Spark: {}", printable(&estimate.spark));
    println!("  SOGNI: {}", printable(&estimate.sogni));
    println!("  USD:   {}", printable(&estimate.usd));
    if let Some(seconds) = estimate.estimated_total_seconds {
        println!("  Estimated total time: {seconds:.1}s");
    }
    require_confirmation("Submit this paid request?", assume_yes)
}

/// Resolve payment token from CLI, environment, an optional prompt, then Spark.
pub fn resolve_token_type(explicit: Option<TokenType>, interactive: bool) -> Result<TokenType> {
    if let Some(value) = explicit.or(TokenType::from_environment()?) {
        return Ok(value);
    }
    if !interactive {
        return Ok(TokenType::Spark);
    }
    let value = prompt("Payment token (spark/sogni)", Some("spark"))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "spark" | "1" => Ok(TokenType::Spark),
        "sogni" | "2" => Ok(TokenType::Sogni),
        _ => bail!("payment token must be spark or sogni"),
    }
}

pub fn select(label: &str, choices: &[(&str, &str)], default_key: &str) -> Result<String> {
    println!("{label}:");
    for (index, (key, description)) in choices.iter().enumerate() {
        let marker = if *key == default_key {
            " (default)"
        } else {
            ""
        };
        println!("  {}. {key}: {description}{marker}", index + 1);
    }
    let default_index = choices
        .iter()
        .position(|(key, _)| *key == default_key)
        .unwrap_or(0)
        + 1;
    let answer = prompt("Enter choice", Some(&default_index.to_string()))?;
    if let Ok(index) = answer.parse::<usize>() {
        return choices
            .get(index.saturating_sub(1))
            .map(|(key, _)| (*key).to_owned())
            .ok_or_else(|| anyhow::anyhow!("choice must be between 1 and {}", choices.len()));
    }
    choices
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(&answer))
        .map(|(key, _)| (*key).to_owned())
        .ok_or_else(|| anyhow::anyhow!("unknown choice {answer:?}"))
}

fn printable(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

/// Resolve mutually exclusive inline/file prompt input and reject blank prompts.
pub fn prompt_text(inline: Option<String>, file: Option<&Path>, fallback: &str) -> Result<String> {
    if inline.is_some() && file.is_some() {
        bail!("prompt text and --prompt-file are mutually exclusive");
    }
    let prompt = if let Some(path) = file {
        fs::read_to_string(path)
            .with_context(|| format!("read prompt file {}", path.display()))?
            .trim()
            .to_owned()
    } else {
        inline.unwrap_or_else(|| fallback.to_owned())
    };
    if prompt.trim().is_empty() {
        bail!("prompt cannot be empty");
    }
    Ok(prompt)
}

pub fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn validate_dimensions(width: u32, height: u32, minimum: u32, maximum: u32) -> Result<()> {
    if !(minimum..=maximum).contains(&width) || !(minimum..=maximum).contains(&height) {
        bail!("width and height must each be between {minimum} and {maximum}");
    }
    Ok(())
}
