use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sogni_client::{WorkflowBillingOptions, WorkflowStart};

use crate::common;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Action {
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
struct Args {
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
    /// Explicit action selector; the upstream-style --list/--get/... flags are also accepted.
    #[arg(long, value_enum)]
    action: Option<Action>,
    #[arg(long)]
    workflow_id: Option<String>,
    #[arg(long)]
    list: bool,
    #[arg(long, value_name = "WORKFLOW_ID")]
    get: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    events: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    stream: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    cancel: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    resume: Option<String>,
    #[arg(long, value_name = "WORKFLOW_ID")]
    reseed: Option<String>,
    #[arg(long)]
    watch: bool,
    #[arg(long)]
    after: Option<String>,
    #[arg(long)]
    last_event_id: Option<String>,
    #[arg(long)]
    video_prompt: Option<String>,
    #[arg(long)]
    negative_prompt: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long, default_value_t = 5.0)]
    duration: f64,
    #[arg(long, default_value = "gpt-image-2")]
    image_model: String,
    #[arg(long, default_value = "ltx23")]
    video_model: String,
    #[arg(long = "number", default_value_t = 1)]
    number_of_media: u32,
    #[arg(long)]
    seed: Option<i64>,
    #[arg(long, default_value = "spark")]
    token_type: String,
    #[arg(long, default_value = "auto")]
    billing_mode: String,
    /// Perform the authenticated network operation.
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let action = selected_action(&args)?;
    validate(&args, action)?;
    let request = start_request(&args);
    if !args.execute || args.dry_run {
        println!("Dry run; pass --execute to perform this network operation.\nAction: {action:?}");
        if action == Action::Start {
            println!("{}", serde_json::to_string_pretty(&request)?);
        } else if action != Action::List {
            println!("Workflow: {}", required_id(&args, action)?);
        }
        return Ok(());
    }
    let credentials = common::auth::load_credentials()?;
    let client = common::auth::connect_api_key_rest_only(
        common::auth::unique_app_id("sogni-creative-workflows"),
        credentials,
    )
    .await?;
    let outcome = dispatch(&client, &args, action, request).await;
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

async fn dispatch(
    client: &sogni_client::SogniClient,
    args: &Args,
    action: Action,
    start: WorkflowStart,
) -> Result<()> {
    match action {
        Action::List => {
            for workflow in client.workflows.list(Some(20), None).await? {
                print_workflow(&workflow);
            }
        }
        Action::Get => print_workflow(&client.workflows.get(required_id(args, action)?).await?),
        Action::Events => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client.workflows.events(required_id(args, action)?).await?
                )?
            );
        }
        Action::Stream => stream(client, required_id(args, action)?, args).await?,
        Action::Cancel => {
            print_workflow(&client.workflows.cancel(required_id(args, action)?).await?)
        }
        Action::Resume => {
            let value = client
                .workflows
                .resume(required_id(args, action)?, billing(args))
                .await?;
            println!("Resumed: {}", value.resumed);
            print_workflow(&value.workflow);
            if args.watch {
                stream(client, required_id(args, action)?, args).await?;
            }
        }
        Action::Reseed => {
            let value = client
                .workflows
                .reseed(required_id(args, action)?, billing(args), None)
                .await?;
            println!("Cloned from workflow: {}", value.reseed.cloned_from_run_id);
            for step in &value.reseed.steps {
                println!("  reseeded step: {step}");
            }
            print_workflow(&value.workflow);
            if args.watch {
                stream(client, required_id(args, action)?, args).await?;
            }
        }
        Action::Start => {
            if args.prompt.join(" ").trim().is_empty() {
                bail!("a workflow prompt is required");
            }
            let workflow = client.workflows.start(start).await?;
            print_workflow(&workflow);
            if args.watch {
                let id = workflow_id(&workflow).context("start response omitted workflow id")?;
                stream(client, id, args).await?;
            }
        }
    }
    Ok(())
}

fn start_request(args: &Args) -> WorkflowStart {
    let prompt = args.prompt.join(" ");
    WorkflowStart {
        input: Some(json!({
            "title": "Generated keyframe to video",
            "steps": [
                {
                    "id": "keyframe",
                    "toolName": "generate_image",
                    "arguments": {
                        "prompt": prompt,
                        "negativePrompt": args.negative_prompt,
                        "width": args.width,
                        "height": args.height,
                        "model": args.image_model,
                        "numberOfVariations": args.number_of_media,
                        "seed": args.seed,
                    }
                },
                {
                    "id": "clip",
                    "toolName": "generate_video",
                    "arguments": {
                        "prompt": args.video_prompt.as_deref().unwrap_or(&prompt),
                        "negativePrompt": args.negative_prompt,
                        "width": args.width,
                        "height": args.height,
                        "duration": args.duration,
                        "videoModel": args.video_model,
                        "numberOfVariations": args.number_of_media,
                    },
                    "dependsOn": [{
                        "sourceStepId": "keyframe",
                        "sourceArtifactIndex": 0,
                        "targetArgument": "referenceImageIndices",
                        "mediaType": "image",
                        "transform": "image_index",
                        "required": true,
                    }]
                }
            ]
        })),
        token_type: Some(args.token_type.clone()),
        billing_mode: Some(args.billing_mode.clone()),
        app_source: Some("sogni-client-rs-example".into()),
        ..WorkflowStart::default()
    }
}

fn billing(args: &Args) -> WorkflowBillingOptions {
    WorkflowBillingOptions {
        token_type: Some(args.token_type.clone()),
        billing_mode: Some(args.billing_mode.clone()),
        app_source: Some("sogni-client-rs-example".into()),
        ..WorkflowBillingOptions::default()
    }
}

async fn stream(client: &sogni_client::SogniClient, id: &str, args: &Args) -> Result<()> {
    println!(
        "Streaming events for {id}; resume cursor {:?}",
        args.last_event_id
    );
    let mut stream = client
        .workflows
        .stream_events(id, args.after.as_deref(), args.last_event_id.as_deref())
        .await?;
    while let Some(frame) = stream.next().await {
        let frame = frame?;
        println!(
            "[{}] {} {}",
            frame.id.as_deref().unwrap_or("-"),
            frame.event,
            frame.data
        );
    }
    Ok(())
}

fn required_id(args: &Args, action: Action) -> Result<&str> {
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

fn selected_action(args: &Args) -> Result<Action> {
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

fn validate(args: &Args, action: Action) -> Result<()> {
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

fn workflow_id(value: &Value) -> Option<&str> {
    value
        .get("workflowId")
        .or_else(|| value.get("workflow_id"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}

fn print_workflow(value: &Value) {
    println!(
        "Workflow: {}\nStatus:   {}",
        workflow_id(value).unwrap_or("unknown"),
        value.get("status").and_then(Value::as_str).unwrap_or("-")
    );
    if let Some(artifacts) = value.get("artifacts").and_then(Value::as_array) {
        for artifact in artifacts {
            println!("  artifact: {artifact}");
        }
    }
}

trait OptionContext<T> {
    fn context(self, message: &str) -> Result<T>;
}

impl<T> OptionContext<T> for Option<T> {
    fn context(self, message: &str) -> Result<T> {
        self.ok_or_else(|| anyhow::anyhow!("{message}"))
    }
}
