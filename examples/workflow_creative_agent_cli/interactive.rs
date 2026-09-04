use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
};

use anyhow::{Context, Result};
use sogni_client::SogniClient;

use crate::{
    common, response, runner,
    session::{self, Session, ToolsMode},
};

pub async fn run(client: &SogniClient, session: &mut Session) -> Result<()> {
    let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    let input = io::stdin();
    loop {
        if terminal {
            print!("sogni> ");
            io::stdout().flush()?;
        }
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('/') {
            if !command(client, session, line).await? {
                break;
            }
        } else if matches!(line, "exit" | "quit") {
            break;
        } else if let Err(error) = runner::run_turn(client, session, line).await {
            eprintln!("Error: {error:#}");
        }
    }
    Ok(())
}

async fn command(client: &SogniClient, session: &mut Session, line: &str) -> Result<bool> {
    let body = line.trim_start_matches('/').trim();
    let (name, argument) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    let argument = argument.trim();
    match name.to_ascii_lowercase().as_str() {
        "help" | "?" => print_help(),
        "exit" | "quit" | "q" => return Ok(false),
        "context" => session::print_context_details(session),
        "reload" => {
            session.reload_context();
            session::print_context_details(session);
        }
        "add" if argument.is_empty() => println!("Usage: /add <markdown-file|directory|glob>"),
        "add" => {
            session.add_context_source(argument.into());
            session::print_context_details(session);
        }
        "system" => set_system(session, argument),
        "tools" => set_tools(session, argument),
        "execute" => set_execution(session, argument),
        "billing" => set_billing(session, argument),
        "token" => set_token(session, argument),
        "model" => set_model(session, argument),
        "subscription" => runner::print_subscription(client).await,
        "history" => response::print_history(session),
        "clear" => {
            session.clear();
            println!("History cleared.");
        }
        "last" => response::print_last(session, argument == "--json"),
        "save" => save_transcript(session, argument)?,
        "config" => session::print_config(session),
        _ => println!("Unknown command: /{name}. Type /help for commands."),
    }
    Ok(true)
}

fn set_system(session: &mut Session, argument: &str) {
    if argument.is_empty() {
        println!(
            "{}",
            if session.options.session_instruction.is_empty() {
                "(No extra session instruction set.)"
            } else {
                &session.options.session_instruction
            }
        );
    } else if argument.eq_ignore_ascii_case("clear") {
        session.options.session_instruction.clear();
        println!("Extra session instruction cleared.");
    } else {
        session.options.session_instruction = argument.into();
        println!("Extra session instruction updated.");
    }
}

fn set_tools(session: &mut Session, argument: &str) {
    if !argument.is_empty() {
        session.options.tools_mode = ToolsMode::parse(argument)
    }
    println!("Tools mode: {}", session.options.tools_mode.label());
}

fn set_execution(session: &mut Session, argument: &str) {
    match argument.to_ascii_lowercase().as_str() {
        "" => {}
        "on" | "true" | "yes" | "1" => session.options.execute_tools = true,
        "off" | "false" | "no" | "0" => session.options.execute_tools = false,
        _ => {
            println!("Usage: /execute on|off");
            return;
        }
    }
    println!(
        "Tool execution: {}",
        if session.options.execute_tools {
            "on"
        } else {
            "off"
        }
    );
}

fn set_billing(session: &mut Session, argument: &str) {
    if argument.is_empty() {
        println!("Billing mode: {}", session.options.billing_mode);
    } else {
        match session::validate_billing(argument) {
            Ok(value) => {
                session.options.billing_mode = value;
                println!("Billing mode: {}", session.options.billing_mode);
            }
            Err(error) => println!("{error}"),
        }
    }
}

fn set_token(session: &mut Session, argument: &str) {
    if argument.is_empty() {
        println!("Token type: {}", session.options.token_type);
    } else {
        match session::validate_token(argument) {
            Ok(value) => {
                session.options.token_type = value;
                println!("Token type: {}", session.options.token_type);
            }
            Err(error) => println!("{error}"),
        }
    }
}

fn set_model(session: &mut Session, argument: &str) {
    if !argument.is_empty() {
        session.options.model = argument.into()
    }
    println!("Model: {}", session.options.model);
}

fn save_transcript(session: &Session, argument: &str) -> Result<()> {
    let desired = if argument.is_empty() {
        session.options.output_dir.join(format!(
            "creative-agent-session-{}.md",
            chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ")
        ))
    } else {
        let path = PathBuf::from(argument);
        if path.is_absolute() {
            path
        } else {
            session.options.workspace.join(path)
        }
    };
    let target = common::files::unique_path(desired);
    if let Some(parent) = target.parent() {
        common::files::ensure_output_dir(parent)?
    }
    let mut output = format!(
        "# Sogni Creative Agent CLI Session\n\nSaved: {}\nModel: {}\nTools: {}\nTool execution: {}\nBilling mode: {}\n\n## Context Files\n\n",
        chrono::Utc::now().to_rfc3339(),
        session.options.model,
        session.options.tools_mode.label(),
        if session.options.execute_tools {
            "on"
        } else {
            "off"
        },
        session.options.billing_mode
    );
    if session.context.docs.is_empty() {
        output.push_str("- none\n")
    }
    for doc in &session.context.docs {
        output.push_str(&format!("- {}\n", doc.relative_path))
    }
    output.push_str("\n## Transcript\n\n");
    for turn in &session.turns {
        output.push_str(&format!(
            "### User\n\n{}\n\n### Assistant\n\n{}\n\n",
            turn.user_text,
            response::assistant_history(&turn.response)
        ));
    }
    fs::write(&target, output).with_context(|| format!("write transcript {}", target.display()))?;
    println!("Saved transcript: {}", target.display());
    Ok(())
}

fn print_help() {
    println!(
        r#"
Commands:
  /context              Show loaded Markdown files and warnings
  /reload               Reload Markdown context from disk
  /add <path>           Add a Markdown file, directory, or glob
  /system [text|clear]  Show, set, or clear extra session instruction
  /tools [mode]         Show or set creative-agent, creative-tools, true, or none
  /execute [on|off]     Toggle server-side Sogni tool execution
  /billing [mode]       Show or set auto, subscription, or tokens
  /token [spark|sogni]  Show or set token type
  /model [id]           Show or set LLM model
  /subscription         Fetch current subscription status
  /history              Show compact chat history
  /clear                Clear chat history
  /last [--json]        Show last response summary or raw hosted JSON
  /save [path]          Save the durable transcript as Markdown
  /config               Show runtime configuration
  /exit                 Quit
"#
    );
}
