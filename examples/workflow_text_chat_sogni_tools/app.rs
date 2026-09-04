use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{
    ChatToolCall, Network, SogniClient, is_sogni_tool_call, parse_tool_call_arguments,
};

use crate::{
    common, composition, generation, schemas, shared,
    types::{GeneratedMedia, MediaKind, PipelineConfig, number_arg, quantity_arg, string_arg},
};

const DEFAULT_IMAGE_MODEL: &str = "z_image_turbo_bf16";
const DEFAULT_VIDEO_MODEL: &str = "ltx23-22b-fp8_t2v_distilled";
const DEFAULT_AUDIO_MODEL: &str = "ace_step_1.5_xl_turbo";
const DEFAULT_SYSTEM: &str = "You are a creative assistant with access to the Sogni Supernet. For requests to create images, videos, or music, immediately call the matching tool and pass through the user's raw intent without embellishment. For ordinary conversation, answer normally. After generation, briefly summarize the result without URLs or download paths.";

#[derive(Parser)]
#[command(about = "LLM-routed Sogni image, video, and music composition pipelines")]
struct Args {
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
    #[arg(long, default_value = shared::runtime::DEFAULT_MODEL)]
    model: String,
    #[arg(long, default_value = DEFAULT_IMAGE_MODEL)]
    image_model: String,
    #[arg(long, default_value = DEFAULT_VIDEO_MODEL)]
    video_model: String,
    #[arg(long, default_value = DEFAULT_AUDIO_MODEL)]
    audio_model: String,
    #[arg(long)]
    max_tokens: Option<u64>,
    #[arg(long)]
    temperature: Option<f64>,
    #[arg(long)]
    top_p: Option<f64>,
    #[arg(long)]
    top_k: Option<u64>,
    #[arg(long, default_value = DEFAULT_SYSTEM)]
    system: String,
    #[arg(long)]
    no_think: bool,
    #[arg(long)]
    show_thinking: bool,
    #[arg(short = 'n', long, default_value_t = 1)]
    quantity: u32,
    #[arg(long)]
    duration: Option<f64>,
    #[arg(long = "aspect-ratio", alias = "ar", default_value = "portrait")]
    aspect_ratio: String,
    #[arg(long, default_value_t = shared::default_token_type())]
    token_type: String,
    #[arg(long, default_value_t = shared::default_billing_mode())]
    billing_mode: String,
    #[arg(long, default_value = "examples/output/sogni-tools")]
    output_dir: PathBuf,
    /// Accept every displayed estimate and confirmation.
    #[arg(long)]
    yes: bool,
    /// Submit paid LLM and media-generation requests.
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    validate(&args)?;
    let prompt = if args.prompt.is_empty() {
        "Create a cinematic image of a bioluminescent forest at dusk".into()
    } else {
        args.prompt.join(" ")
    };
    let mut settings = shared::ChatSettings {
        model: args.model,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        top_p: args.top_p,
        top_k: args.top_k,
        think: !args.no_think,
        task_profile: "reasoning".into(),
        token_type: args.token_type.clone(),
        billing_mode: args.billing_mode.clone(),
        ..shared::ChatSettings::default()
    };
    shared::validate_settings(&settings)?;
    let messages = vec![
        json!({"role": "system", "content": args.system}),
        json!({"role": "user", "content": prompt}),
    ];
    if !common::cli::execution_requested(args.execute, args.dry_run)? {
        let mut request = shared::runtime::request(&settings, &messages, true);
        request["tools"] = Value::Array(schemas::intent_tools());
        request["tool_choice"] = json!("auto");
        println!("Dry-run intent request:\n{request:#}");
        println!(
            "Pipeline models: image={}, video={}, audio={}",
            args.image_model, args.video_model, args.audio_model
        );
        common::cli::explain_dry_run();
        return Ok(());
    }
    if args.prompt.is_empty() {
        bail!("a prompt is required with --execute");
    }
    let config = PipelineConfig {
        image_model: args.image_model,
        video_model: args.video_model,
        audio_model: args.audio_model,
        quantity: args.quantity,
        duration: args.duration,
        aspect_ratio: normalize_aspect(&args.aspect_ratio)?,
        output_dir: args.output_dir,
        assume_yes: args.yes,
        show_thinking: args.show_thinking,
        token_type: args.token_type,
        billing_mode: args.billing_mode,
    };
    let client = common::auth::connect(
        common::auth::unique_app_id("sogni-platform-tools"),
        Network::Fast,
    )
    .await?;
    shared::resolve_model_defaults(&client, &mut settings).await;
    if let Err(error) = client
        .projects
        .wait_for_models(Duration::from_secs(15))
        .await
    {
        eprintln!("Warning: media model catalog unavailable: {error}");
    }
    shared::estimate_and_print(&client, &settings, &messages).await;
    common::cli::require_confirmation("Submit paid intent-routing LLM request?", args.yes)?;
    let mut request = shared::runtime::request(&settings, &messages, true);
    request["tools"] = Value::Array(schemas::intent_tools());
    // Routing is optional: ordinary conversation can finish without a tool call.
    // The specialized composition stage uses `required` instead.
    request["tool_choice"] = json!("auto");
    let reporter = shared::spawn_chat_reporter(&client);
    let outcome = run_pipeline(&client, &settings, messages, request, &config).await;
    reporter.abort();
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

async fn run_pipeline(
    client: &SogniClient,
    settings: &shared::ChatSettings,
    messages: Vec<Value>,
    request: Value,
    config: &PipelineConfig,
) -> Result<()> {
    let started = Instant::now();
    print!("\nAssistant: ");
    let (content, result, ttft) =
        shared::stream_response(client, &request, config.show_thinking).await?;
    println!();
    shared::completion_summary(&result, started.elapsed(), Some(ttft));
    if result.tool_calls.is_empty() {
        return Ok(());
    }
    println!(
        "\nThe model requested {} media tool call(s).",
        result.tool_calls.len()
    );
    let mut tool_messages = Vec::new();
    for call in &result.tool_calls {
        tool_messages.push(handle_call(client, settings, call, config).await);
    }
    let mut follow_up = messages;
    // Preserve protocol ordering: the assistant call declaration comes before
    // the tool results carrying its ids, then the model receives the full history.
    follow_up.push(json!({
        "role": "assistant",
        "content": if content.is_empty() { Value::Null } else { json!(content) },
        "tool_calls": result.tool_calls,
    }));
    follow_up.extend(tool_messages);
    let follow_request = shared::runtime::request(settings, &follow_up, true);
    print!("\nAssistant: ");
    let summary_started = Instant::now();
    let (_, summary, summary_ttft) =
        shared::stream_response(client, &follow_request, config.show_thinking).await?;
    println!();
    shared::completion_summary(&summary, summary_started.elapsed(), Some(summary_ttft));
    Ok(())
}

async fn handle_call(
    client: &SogniClient,
    settings: &shared::ChatSettings,
    call: &ChatToolCall,
    config: &PipelineConfig,
) -> Value {
    let call_value = serde_json::to_value(call).unwrap_or(Value::Null);
    let result = execute_call(client, settings, call, &call_value, config).await;
    let content = match result {
        Ok(media) => tool_success(&media),
        Err(error) => {
            eprintln!("Media pipeline failed: {error:#}");
            json!({"success": false, "error": error.to_string()})
        }
    };
    json!({"role": "tool", "tool_call_id": call.id, "name": call.function.name, "content": content.to_string()})
}

async fn execute_call(
    client: &SogniClient,
    settings: &shared::ChatSettings,
    call: &ChatToolCall,
    call_value: &Value,
    config: &PipelineConfig,
) -> Result<GeneratedMedia> {
    if !is_sogni_tool_call(call_value) {
        bail!("unknown tool {}", call.function.name);
    }
    let kind = MediaKind::from_tool(&call.function.name).with_context(|| {
        format!(
            "tool {} is not a media-generation intent",
            call.function.name
        )
    })?;
    let args = parse_tool_call_arguments(call_value);
    let intent = string_arg(&args, "prompt").context("media tool call omitted prompt")?;
    let quantity = quantity_arg(&args, config.quantity);
    let duration = match kind {
        MediaKind::Video => number_arg(&args, "duration")
            .or(config.duration)
            .unwrap_or(10.0)
            .clamp(1.0, 20.0),
        MediaKind::Audio => number_arg(&args, "duration")
            .or(config.duration)
            .unwrap_or(30.0)
            .clamp(10.0, 600.0),
        MediaKind::Image => 0.0,
    };
    let aspect = string_arg(&args, "aspect_ratio")
        .and_then(|value| normalize_aspect(&value).ok())
        .unwrap_or_else(|| config.aspect_ratio.clone());
    println!(
        "\nTool: {}\nIntent: {intent}\nQuantity: {quantity}",
        call.function.name
    );
    let spec = composition::compose(
        client,
        kind,
        &intent,
        duration,
        settings,
        config.assume_yes,
        config.show_thinking,
    )
    .await?;
    generation::generate(
        client,
        spec,
        config,
        quantity,
        Some(duration).filter(|_| kind != MediaKind::Image),
        Some(&aspect),
    )
    .await
}

fn tool_success(media: &GeneratedMedia) -> Value {
    println!("Generated {} with {}:", media.kind.label(), media.model);
    for file in &media.files {
        println!("  {}", file.display());
    }
    json!({
        "success": true,
        "media_type": media.kind.label(),
        "model": media.model,
        "prompt": media.prompt,
        "local_file": media.files.first().map(|path| path.display().to_string()),
        "files_saved": media.files.len(),
        "note": "Files are saved locally. Do not include URLs or download paths in the final response."
    })
}

fn validate(args: &Args) -> Result<()> {
    if !(1..=512).contains(&args.quantity) {
        bail!("--quantity must be between 1 and 512")
    }
    if let Some(duration) = args
        .duration
        .filter(|value| !value.is_finite() || *value <= 0.0)
    {
        bail!("--duration must be a finite positive number, got {duration}")
    }
    for (flag, value) in [
        ("--model", &args.model),
        ("--image-model", &args.image_model),
        ("--video-model", &args.video_model),
        ("--audio-model", &args.audio_model),
    ] {
        if value.trim().is_empty() {
            bail!("{flag} cannot be empty");
        }
    }
    normalize_aspect(&args.aspect_ratio)?;
    Ok(())
}

fn normalize_aspect(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase().replace([':', ' '], "_");
    let normalized = match normalized.as_str() {
        "vertical" | "9_16" | "tall" => "portrait",
        "horizontal" | "16_9" => "landscape",
        "cinematic" | "wide" => "widescreen",
        "4_3" => "landscape_4_3",
        "3_4" => "portrait_4_3",
        "1_1" => "square",
        other => other,
    };
    if matches!(
        normalized,
        "portrait" | "landscape" | "widescreen" | "square" | "portrait_4_3" | "landscape_4_3"
    ) {
        Ok(normalized.into())
    } else {
        bail!("unsupported aspect ratio {value:?}")
    }
}
