mod image;

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{Network, SogniClient};

use self::image::LoadedImage;
use crate::{common, shared};

const DEFAULT_SYSTEM: &str = "You are a visual analysis assistant with expert-level image understanding. Analyze images thoroughly and be precise about colors, positions, text, objects, and spatial relationships.";
const DESCRIBE: &str = "Provide a rich, detailed description of this image, including subject, background, colors, lighting, mood, visible text, spatial relationships, and composition.";
const OCR: &str = "Extract ALL visible text. Preserve layout where possible, identify language and approximate location, and include partially visible or stylized text.";
const OBJECTS: &str = "List every identifiable object with location, relative size, visual attributes, and spatial relationships.";
const ANALYZE: &str = "Analyze SUBJECT, COMPOSITION, LIGHTING, COLOR, TECHNICAL qualities, MOOD, STYLE, and CONTEXT in a clear structured response.";
const COMPARE: &str = "Compare Image 1 and Image 2 in subject, color, lighting, composition, mood, quality, and other notable similarities and differences.";

#[derive(Parser)]
#[command(about = "Interactive multimodal Sogni vision chat")]
struct Args {
    #[arg(long)]
    image: Option<String>,
    #[arg(long, default_value = shared::runtime::DEFAULT_MODEL)]
    model: String,
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
    #[arg(long, default_value_t = shared::default_token_type())]
    token_type: String,
    #[arg(long, default_value_t = shared::default_billing_mode())]
    billing_mode: String,
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

#[derive(Default)]
struct Stats {
    turns: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    elapsed: Duration,
}

struct Session {
    system: String,
    image: Option<LoadedImage>,
    history: Vec<Value>,
    stats: Stats,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let loaded = args.image.as_deref().map(image::load).transpose()?;
    let mut settings = shared::ChatSettings {
        model: args.model,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        top_p: args.top_p,
        top_k: args.top_k,
        think: false,
        task_profile: "reasoning".into(),
        token_type: args.token_type,
        billing_mode: args.billing_mode,
        ..shared::ChatSettings::default()
    };
    shared::validate_settings(&settings)?;
    if !args.execute || args.dry_run {
        println!("Dry run; pass --execute to start paid vision chat.");
        println!("Model: {}\nSystem: {}", settings.model, args.system);
        if let Some(image) = loaded {
            print_image(&image);
        }
        return Ok(());
    }
    let client =
        common::auth::connect(common::auth::unique_app_id("sogni-vision"), Network::Fast).await?;
    shared::resolve_model_defaults(&client, &mut settings).await;
    let reporter = shared::spawn_chat_reporter(&client);
    let mut session = Session {
        system: args.system,
        image: loaded,
        history: Vec::new(),
        stats: Stats::default(),
    };
    println!("Commands: /image PATH, /describe, /ocr, /objects, /analyze, /compare PATH,");
    println!("          /clear-image, /clear, /history, /system TEXT, /stats, exit");
    let result = chat_loop(&client, &settings, &mut session).await;
    reporter.abort();
    print_stats(&session);
    let close = common::auth::close(&client).await;
    result?;
    close
}

async fn chat_loop(
    client: &SogniClient,
    settings: &shared::ChatSettings,
    session: &mut Session,
) -> Result<()> {
    loop {
        let label = session
            .image
            .as_ref()
            .map(|image| format!("You [{}]: ", image.file_name))
            .unwrap_or_else(|| "You: ".into());
        let input = shared::prompt_line(&label)?;
        if input.is_empty() {
            continue;
        }
        if matches!(input.to_ascii_lowercase().as_str(), "exit" | "quit") {
            break;
        }
        let Some((text, comparison)) = command(&input, session)? else {
            continue;
        };
        let message = user_message(&text, session.image.as_ref(), comparison.as_ref());
        session.history.push(message);
        let mut messages = vec![json!({"role": "system", "content": session.system})];
        messages.extend(session.history.iter().cloned());
        shared::estimate_and_print(client, settings, &messages).await;
        let request = shared::runtime::request(settings, &messages, true);
        print!("\nAssistant: ");
        let started = Instant::now();
        match shared::stream_response(client, &request, false).await {
            Ok((raw, result, ttft)) => {
                let clean = shared::strip_thinking(&raw);
                session.history.push(json!({
                    "role": "assistant",
                    "content": if clean.is_empty() { raw } else { clean },
                }));
                session.stats.turns += 1;
                session.stats.elapsed += started.elapsed();
                session.stats.prompt_tokens += usage(&result.usage, "prompt_tokens");
                session.stats.completion_tokens += usage(&result.usage, "completion_tokens");
                println!(
                    "\n  [TTFT {:.2}s | {:.2}s | {} prompt + {} completion tokens]\n",
                    ttft.as_secs_f64(),
                    started.elapsed().as_secs_f64(),
                    usage(&result.usage, "prompt_tokens"),
                    usage(&result.usage, "completion_tokens")
                );
            }
            Err(error) => {
                // Keep the multimodal transcript pairwise: failed user/image
                // messages must not leak into the next request as answered turns.
                session.history.pop();
                eprintln!("\nError: {error}\n");
            }
        }
    }
    Ok(())
}

fn command(input: &str, session: &mut Session) -> Result<Option<(String, Option<LoadedImage>)>> {
    if !input.starts_with('/') {
        return Ok(Some((input.into(), None)));
    }
    let (name, argument) = input.split_once(' ').unwrap_or((input, ""));
    let require_image =
        |prompt: &str, session: &Session| session.image.as_ref().map(|_| (prompt.to_owned(), None));
    let request = match name.to_ascii_lowercase().as_str() {
        "/image" => {
            session.image = Some(image::load(argument.trim())?);
            if let Some(image) = &session.image {
                print_image(image);
            }
            None
        }
        "/describe" => require_image(DESCRIBE, session),
        "/ocr" => require_image(OCR, session),
        "/objects" => require_image(OBJECTS, session),
        "/analyze" => require_image(ANALYZE, session),
        "/compare" => {
            if session.image.is_none() {
                eprintln!("Load the first image with /image PATH.");
                None
            } else {
                Some((COMPARE.into(), Some(image::load(argument.trim())?)))
            }
        }
        "/clear-image" => {
            session.image = None;
            println!("Image removed from context.");
            None
        }
        "/clear" => {
            session.history.clear();
            println!("Conversation history cleared.");
            None
        }
        "/history" => {
            print_history(session);
            None
        }
        "/system" => {
            if argument.trim().is_empty() {
                println!("System: {}", session.system);
            } else {
                session.system = argument.trim().into();
                println!("System prompt updated.");
            }
            None
        }
        "/stats" => {
            print_stats(session);
            None
        }
        _ => {
            eprintln!("Unknown command: {name}");
            None
        }
    };
    if request.is_none() && matches!(name, "/describe" | "/ocr" | "/objects" | "/analyze") {
        eprintln!("No image loaded. Use /image PATH.");
    }
    Ok(request)
}

fn user_message(
    text: &str,
    image: Option<&LoadedImage>,
    comparison: Option<&LoadedImage>,
) -> Value {
    let Some(image) = image else {
        return json!({"role": "user", "content": text});
    };
    let mut content = vec![json!({"type": "image_url", "image_url": {"url": image.data_uri}})];
    if let Some(comparison) = comparison {
        content.push(json!({"type": "image_url", "image_url": {"url": comparison.data_uri}}));
    }
    content.push(json!({"type": "text", "text": text}));
    json!({"role": "user", "content": content})
}

fn print_image(image: &LoadedImage) {
    println!(
        "Image loaded: {} ({}, {:.0} KiB)",
        image.file_name,
        image.format,
        image.source_bytes as f64 / 1024.0
    );
}

fn usage(value: &Value, name: &str) -> u64 {
    value.get(name).and_then(Value::as_u64).unwrap_or(0)
}

fn print_stats(session: &Session) {
    println!(
        "Session: {} turns, {} prompt + {} completion tokens, {:.2}s, {} history messages, image {}",
        session.stats.turns,
        session.stats.prompt_tokens,
        session.stats.completion_tokens,
        session.stats.elapsed.as_secs_f64(),
        session.history.len(),
        session
            .image
            .as_ref()
            .map(|value| value.file_name.as_str())
            .unwrap_or("none")
    );
}

fn print_history(session: &Session) {
    println!("System: {}", session.system);
    for message in &session.history {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("?");
        let image = message.get("content").is_some_and(Value::is_array);
        println!(
            "{role}{}: {}",
            if image { " [+image]" } else { "" },
            message.get("content").unwrap_or(&Value::Null)
        );
    }
}
