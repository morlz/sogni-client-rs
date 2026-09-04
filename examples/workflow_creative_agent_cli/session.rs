use std::path::PathBuf;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::context::{self, ContextOptions, LoadedContext};

pub const DEFAULT_MODEL: &str = "qwen3.6-35b-a3b-gguf-iq4xs";
pub const BASE_SYSTEM: &str = "You are Sogni Creative Agent running inside a local CLI. Help the customer produce concrete image, video, music, and workflow outputs using Sogni creative tools. Use the local Markdown context as persistent customer training for style preferences, shorthand, recurring subjects, brand rules, art direction, and production constraints. Honor it unless the current turn overrides it. Never claim access to arbitrary local files: you only know the supplied Markdown context and conversation. After a tool generates or submits media, concisely summarize the result and next useful creative action.";

#[derive(Clone, Debug)]
pub enum ToolsMode {
    Named(String),
    Enabled,
    Disabled,
}

impl ToolsMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "creative-agent" => Self::Named("creative-agent".into()),
            "creative-tools" | "rich" | "hosted" => Self::Named("creative-tools".into()),
            "true" | "on" | "yes" => Self::Enabled,
            "false" | "none" | "off" | "no" => Self::Disabled,
            _ => Self::Named(value.trim().to_owned()),
        }
    }

    pub fn value(&self) -> Value {
        match self {
            Self::Named(value) => json!(value),
            Self::Enabled => json!(true),
            Self::Disabled => json!(false),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Named(value) => value,
            Self::Enabled => "true",
            Self::Disabled => "none",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub model: String,
    pub tools_mode: ToolsMode,
    pub execute_tools: bool,
    pub token_type: String,
    pub billing_mode: String,
    pub max_tokens: usize,
    pub temperature: f64,
    pub top_p: Option<f64>,
    pub think: bool,
    pub context_sources: Vec<String>,
    pub auto_context: bool,
    pub max_context_chars: usize,
    pub max_file_chars: usize,
    pub max_history: usize,
    pub workspace: PathBuf,
    pub session_instruction: String,
    pub json: bool,
    pub app_source: String,
    pub output_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Turn {
    pub user_text: String,
    pub response: Value,
    pub elapsed_seconds: f64,
}

#[derive(Debug)]
pub struct Session {
    pub options: SessionOptions,
    pub context: LoadedContext,
    pub messages: Vec<Value>,
    pub turns: Vec<Turn>,
    pub last_response: Option<Value>,
}

impl Session {
    pub fn new(options: SessionOptions) -> Self {
        let mut session = Self {
            options,
            context: LoadedContext::default(),
            messages: Vec::new(),
            turns: Vec::new(),
            last_response: None,
        };
        session.reload_context();
        session
    }

    pub fn reload_context(&mut self) {
        self.context = context::load(&ContextOptions {
            workspace: self.options.workspace.clone(),
            sources: self.options.context_sources.clone(),
            auto: self.options.auto_context,
            max_total_chars: self.options.max_context_chars,
            max_file_chars: self.options.max_file_chars,
        });
    }

    pub fn add_context_source(&mut self, source: String) {
        self.options.context_sources.push(source);
        self.reload_context();
    }

    pub fn system_prompt(&self) -> String {
        let mut blocks = vec![BASE_SYSTEM.to_owned()];
        if !self.options.session_instruction.trim().is_empty() {
            blocks.push(format!(
                "Additional session instruction:\n{}",
                self.options.session_instruction.trim()
            ));
        }
        blocks.push(self.formatted_context());
        blocks.join("\n\n")
    }

    fn formatted_context(&self) -> String {
        if self.context.docs.is_empty() {
            return "Local Markdown context: none loaded.".into();
        }
        let mut output = String::from(
            "Local Markdown context:\nTreat these files as customer-provided operating context for style, shortcuts, preferences, and constraints.",
        );
        for doc in &self.context.docs {
            output.push_str(&format!(
                "\n\n--- {} ---\n{}",
                doc.relative_path,
                doc.content.trim()
            ));
        }
        if self.context.budget_exhausted {
            output.push_str("\n\n[Some context was omitted or truncated because the configured budget was exhausted.]");
        }
        output
    }

    pub fn trim_history(&mut self) {
        let remove = self.messages.len().saturating_sub(self.options.max_history);
        if remove > 0 {
            self.messages.drain(..remove);
        }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.turns.clear();
        self.last_response = None;
    }
}

pub fn validate_token(value: &str) -> Result<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "spark" => Ok("spark".into()),
        "sogni" => Ok("sogni".into()),
        _ => bail!("token type must be spark or sogni"),
    }
}

pub fn validate_billing(value: &str) -> Result<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok("auto".into()),
        "subscription" => Ok("subscription".into()),
        "tokens" => Ok("tokens".into()),
        _ => bail!("billing mode must be auto, subscription, or tokens"),
    }
}

pub fn print_startup(session: &Session) {
    println!(
        "{}\n  Sogni Creative Agent CLI\n{}",
        "=".repeat(68),
        "=".repeat(68)
    );
    println!("Model:       {}", session.options.model);
    println!(
        "Tools:       {} ({})",
        session.options.tools_mode.label(),
        if session.options.execute_tools {
            "executing"
        } else {
            "not executing"
        }
    );
    println!("Billing:     {}", session.options.billing_mode);
    println!("Token type:  {}", session.options.token_type);
    println!("Workspace:   {}", session.options.workspace.display());
    print_context_summary(session);
    println!("Type /help for commands, /exit to quit.\n");
}

pub fn print_context_summary(session: &Session) {
    println!(
        "Context:     {} Markdown file(s), {} chars",
        session.context.docs.len(),
        session.context.total_chars
    );
    if session.context.budget_exhausted {
        println!("             context budget exhausted; use /context for details")
    }
}

pub fn print_context_details(session: &Session) {
    println!("\nContext sources:");
    if session.context.sources.is_empty() {
        println!("  none")
    }
    for (index, source) in session.context.sources.iter().enumerate() {
        println!("  {}. {source}", index + 1)
    }
    println!("\nLoaded Markdown files:");
    if session.context.docs.is_empty() {
        println!("  none")
    }
    for (index, doc) in session.context.docs.iter().enumerate() {
        let flags = match (doc.truncated_by_file, doc.truncated_by_total) {
            (true, true) => " (file and total budget truncated)",
            (true, false) => " (file truncated)",
            (false, true) => " (total budget truncated)",
            _ => "",
        };
        println!(
            "  {}. {} - {}/{} chars{flags}",
            index + 1,
            doc.relative_path,
            doc.included_chars,
            doc.original_chars
        );
    }
    println!(
        "Total: {} / {} chars",
        session.context.total_chars, session.options.max_context_chars
    );
    for warning in &session.context.warnings {
        println!("Warning: {warning}")
    }
    println!();
}

pub fn print_config(session: &Session) {
    let options = &session.options;
    println!("\nConfig:");
    println!("  Model:            {}", options.model);
    println!("  Tools:            {}", options.tools_mode.label());
    println!(
        "  Tool execution:   {}",
        if options.execute_tools { "on" } else { "off" }
    );
    println!(
        "  Billing/token:    {} / {}",
        options.billing_mode, options.token_type
    );
    println!("  Max tokens:       {}", options.max_tokens);
    println!("  Temperature:      {}", options.temperature);
    println!(
        "  Top-p:            {}",
        options
            .top_p
            .map_or_else(|| "default".into(), |value| value.to_string())
    );
    println!(
        "  Thinking:         {}",
        if options.think { "on" } else { "off" }
    );
    println!("  Workspace:        {}", options.workspace.display());
    println!(
        "  Context/history:  {} files / {}/{} messages\n",
        session.context.docs.len(),
        session.messages.len(),
        options.max_history
    );
}
