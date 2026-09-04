use anyhow::{Context, Result, bail};
use clap::Parser;
use sogni_client::WorkflowStart;

use crate::{
    cli::{Action, Args, required_id, selected_action, validate},
    common, output, request,
};

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let action = selected_action(&args)?;
    validate(&args, action)?;
    let request = request::start(&args);
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
                output::print_workflow(&workflow);
            }
        }
        Action::Get => {
            output::print_workflow(&client.workflows.get(required_id(args, action)?).await?)
        }
        Action::Events => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &client.workflows.events(required_id(args, action)?).await?
                )?
            );
        }
        Action::Stream => {
            output::stream(
                client,
                required_id(args, action)?,
                args.after.as_deref(),
                args.last_event_id.as_deref(),
            )
            .await?
        }
        Action::Cancel => {
            output::print_workflow(&client.workflows.cancel(required_id(args, action)?).await?)
        }
        Action::Resume => {
            let value = client
                .workflows
                .resume(required_id(args, action)?, request::billing(args))
                .await?;
            println!("Resumed: {}", value.resumed);
            output::print_workflow(&value.workflow);
            if args.watch {
                output::stream(
                    client,
                    required_id(args, action)?,
                    args.after.as_deref(),
                    args.last_event_id.as_deref(),
                )
                .await?;
            }
        }
        Action::Reseed => {
            let value = client
                .workflows
                .reseed(required_id(args, action)?, request::billing(args), None)
                .await?;
            println!("Cloned from workflow: {}", value.reseed.cloned_from_run_id);
            for step in &value.reseed.steps {
                println!("  reseeded step: {step}");
            }
            output::print_workflow(&value.workflow);
            if args.watch {
                output::stream(
                    client,
                    required_id(args, action)?,
                    args.after.as_deref(),
                    args.last_event_id.as_deref(),
                )
                .await?;
            }
        }
        Action::Start => {
            if args.prompt.join(" ").trim().is_empty() {
                bail!("a workflow prompt is required");
            }
            let workflow = client.workflows.start(start).await?;
            output::print_workflow(&workflow);
            if args.watch {
                let id =
                    output::workflow_id(&workflow).context("start response omitted workflow id")?;
                output::stream(
                    client,
                    id,
                    args.after.as_deref(),
                    args.last_event_id.as_deref(),
                )
                .await?;
            }
        }
    }
    Ok(())
}
