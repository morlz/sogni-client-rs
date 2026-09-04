use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::json;

use crate::{
    common, interactive, runner,
    session::{self, Session, SessionOptions, ToolsMode},
};

#[derive(Parser)]
#[command(about = "Hosted Sogni creative-agent CLI with persistent Markdown context")]
struct Args {
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
    #[arg(long, default_value = session::DEFAULT_MODEL)]
    model: String,
    #[arg(long, default_value = "creative-agent")]
    tools: String,
    /// Disable server-side execution while still allowing tool calls.
    #[arg(long)]
    no_execute: bool,
    /// Explicitly enable server-side execution (the default).
    #[arg(long, conflicts_with = "no_execute")]
    execute: bool,
    #[arg(long, default_value_t = default_token_type())]
    token_type: String,
    #[arg(long, default_value_t = default_billing_mode())]
    billing_mode: String,
    #[arg(long, conflicts_with = "tokens")]
    subscription: bool,
    #[arg(long, conflicts_with = "subscription")]
    tokens: bool,
    #[arg(long, default_value_t = 4096)]
    max_tokens: usize,
    #[arg(long, default_value_t = 0.4)]
    temperature: f64,
    #[arg(long)]
    top_p: Option<f64>,
    #[arg(long, conflicts_with = "no_think")]
    think: bool,
    #[arg(long)]
    no_think: bool,
    #[arg(short = 'c', long = "context")]
    context_sources: Vec<String>,
    #[arg(long)]
    no_auto_context: bool,
    #[arg(long, default_value_t = 120_000)]
    max_context_chars: usize,
    #[arg(long, default_value_t = 32_000)]
    max_file_chars: usize,
    #[arg(long, default_value_t = 30)]
    max_history: usize,
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long, default_value = "")]
    system: String,
    #[arg(long)]
    json: bool,
    #[arg(long, default_value = "sogni-creative-agent-cli-example")]
    app_source: String,
    #[arg(long, default_value = "examples/output/creative-agent-cli")]
    output_dir: PathBuf,
    /// Submit paid hosted requests and enable interactive mode.
    #[arg(long)]
    submit: bool,
    #[arg(long, conflicts_with = "submit")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    validate(&args)?;
    let prompt = args.prompt.join(" ").trim().to_owned();
    let workspace = absolute_workspace(&args.workspace)?;
    let billing_mode = if args.subscription {
        "subscription".into()
    } else if args.tokens {
        "tokens".into()
    } else {
        session::validate_billing(&args.billing_mode)?
    };
    let options = SessionOptions {
        model: args.model,
        tools_mode: ToolsMode::parse(&args.tools),
        execute_tools: args.execute || !args.no_execute,
        token_type: session::validate_token(&args.token_type)?,
        billing_mode,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        top_p: args.top_p,
        think: args.think && !args.no_think,
        context_sources: args.context_sources,
        auto_context: !args.no_auto_context,
        max_context_chars: args.max_context_chars,
        max_file_chars: args.max_file_chars,
        max_history: args.max_history,
        workspace,
        session_instruction: args.system,
        json: args.json,
        app_source: args.app_source,
        output_dir: args.output_dir,
    };
    let mut session = Session::new(options);
    session::print_startup(&session);
    if !args.submit || args.dry_run {
        let sample = if prompt.is_empty() {
            "<interactive prompt>"
        } else {
            &prompt
        };
        let request = runner::request(&session, sample);
        println!("Dry-run hosted request (local Markdown content redacted):");
        println!(
            "{}",
            serde_json::to_string_pretty(&redacted_preview(&request, &session))?
        );
        common::cli::explain_dry_run();
        return Ok(());
    }
    let credentials = common::auth::load_credentials()?;
    let client = common::auth::connect_api_key_rest_only(
        common::auth::unique_app_id("sogni-creative-agent-cli"),
        credentials,
    )
    .await?;
    let outcome = if prompt.is_empty() {
        interactive::run(&client, &mut session).await
    } else {
        runner::run_turn(&client, &mut session, &prompt).await
    };
    if let Err(error) = &outcome {
        if runner::is_strict_subscription_error(error) {
            eprintln!(
                "Subscription billing was rejected for this hosted turn. Retry with --billing-mode auto or --tokens; vendor-premium models require Premium Spark billing."
            );
        }
    }
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

fn validate(args: &Args) -> Result<()> {
    if args.max_tokens == 0
        || args.max_context_chars == 0
        || args.max_file_chars == 0
        || args.max_history == 0
    {
        bail!("token, context, file, and history limits must be positive")
    }
    if !args.temperature.is_finite() || !(0.0..=2.0).contains(&args.temperature) {
        bail!("--temperature must be between 0 and 2")
    }
    if args
        .top_p
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        bail!("--top-p must be between 0 and 1")
    }
    Ok(())
}

fn absolute_workspace(path: &PathBuf) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.clone()
    } else {
        env::current_dir()?.join(path)
    };
    let canonical = fs::canonicalize(&absolute)
        .with_context(|| format!("resolve workspace {}", absolute.display()))?;
    if !canonical.is_dir() {
        bail!("workspace is not a directory: {}", canonical.display())
    }
    Ok(canonical)
}

fn redacted_preview(request: &serde_json::Value, session: &Session) -> serde_json::Value {
    let mut preview = request.clone();
    if let Some(messages) = preview
        .get_mut("messages")
        .and_then(serde_json::Value::as_array_mut)
    {
        if let Some(system) = messages.first_mut() {
            system["content"] = json!(format!(
                "[{} Markdown context file(s), {} included chars]",
                session.context.docs.len(),
                session.context.total_chars
            ));
        }
    }
    preview
}

fn default_token_type() -> String {
    env::var("SOGNI_TOKEN_TYPE").unwrap_or_else(|_| "spark".into())
}

fn default_billing_mode() -> String {
    env::var("SOGNI_BILLING_MODE").unwrap_or_else(|_| "auto".into())
}
