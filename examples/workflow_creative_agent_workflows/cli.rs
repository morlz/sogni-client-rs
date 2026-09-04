use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum Action {
    Start,
    List,
    Get,
    Events,
    Stream,
    Cancel,
    Resume,
    Reseed,
}

#[derive(Parser)]
#[command(about = "Start, inspect, resume, stream, reseed, or cancel a durable creative workflow")]
pub(super) struct Args {
    #[arg(trailing_var_arg = true)]
    pub(super) prompt: Vec<String>,
    /// Explicit action selector; the upstream-style --list/--get/... flags are also accepted.
    #[arg(long, value_enum)]
    pub(super) action: Option<Action>,
    #[arg(long)]
    pub(super) workflow_id: Option<String>,
    #[arg(long)]
    pub(super) list: bool,
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub(super) get: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub(super) events: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub(super) stream: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub(super) cancel: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub(super) resume: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub(super) reseed: Option<String>,
    #[arg(long)]
    pub(super) watch: bool,
    #[arg(long)]
    pub(super) after: Option<String>,
    #[arg(long)]
    pub(super) last_event_id: Option<String>,
    #[arg(long)]
    pub(super) video_prompt: Option<String>,
    #[arg(long)]
    pub(super) negative_prompt: Option<String>,
    #[arg(long)]
    pub(super) width: Option<u32>,
    #[arg(long)]
    pub(super) height: Option<u32>,
    #[arg(long, default_value_t = 5.0)]
    pub(super) duration: f64,
    #[arg(long, default_value = "gpt-image-2")]
    pub(super) image_model: String,
    #[arg(long, default_value = "ltx23")]
    pub(super) video_model: String,
    #[arg(long = "number", default_value_t = 1)]
    pub(super) number_of_media: u32,
    #[arg(long)]
    pub(super) seed: Option<i64>,
    #[arg(long, default_value = "spark")]
    pub(super) token_type: String,
    #[arg(long, default_value = "auto")]
    pub(super) billing_mode: String,
    /// Perform the authenticated network operation.
    #[arg(long)]
    pub(super) execute: bool,
    #[arg(long, conflicts_with = "execute")]
    pub(super) dry_run: bool,
}

pub(super) fn selected_action(args: &Args) -> Result<Action> {
    let mut selected = Vec::new();
    if let Some(action) = args.action {
        selected.push(action)
    }
    if args.list {
        selected.push(Action::List)
    }
    for (present, action) in [
        (args.get.is_some(), Action::Get),
        (args.events.is_some(), Action::Events),
        (args.stream.is_some(), Action::Stream),
        (args.cancel.is_some(), Action::Cancel),
        (args.resume.is_some(), Action::Resume),
        (args.reseed.is_some(), Action::Reseed),
    ] {
        if present {
            selected.push(action)
        }
    }
    if selected.len() > 1 {
        bail!("choose only one workflow action")
    }
    Ok(selected.first().copied().unwrap_or(Action::Start))
}

pub(super) fn required_id(args: &Args, action: Action) -> Result<&str> {
    let shortcut = match action {
        Action::Get => args.get.as_deref(),
        Action::Events => args.events.as_deref(),
        Action::Stream => args.stream.as_deref(),
        Action::Cancel => args.cancel.as_deref(),
        Action::Resume => args.resume.as_deref(),
        Action::Reseed => args.reseed.as_deref(),
        _ => None,
    };
    shortcut
        .or(args.workflow_id.as_deref())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("a workflow id is required for {action:?}"))
}

pub(super) fn validate(args: &Args, action: Action) -> Result<()> {
    if action != Action::Start && action != Action::List {
        required_id(args, action)?;
    }
    if !args.duration.is_finite() || !(1.0..=20.0).contains(&args.duration) {
        bail!("--duration must be between 1 and 20 seconds")
    }
    if args.number_of_media == 0 {
        bail!("--number must be at least one")
    }
    if args.width == Some(0) || args.height == Some(0) {
        bail!("--width and --height must be positive")
    }
    if !matches!(
        args.token_type.to_ascii_lowercase().as_str(),
        "spark" | "sogni"
    ) {
        bail!("--token-type must be spark or sogni")
    }
    if !matches!(
        args.billing_mode.to_ascii_lowercase().as_str(),
        "auto" | "subscription" | "tokens"
    ) {
        bail!("--billing-mode must be auto, subscription, or tokens")
    }
    Ok(())
}
