use super::*;
use crate::utils::{is_gpt_image_26_model, is_seedream_model};

pub(super) fn plan(
    tool: &str,
    args: &Value,
    options: &Value,
    models: &[Value],
) -> Result<ToolRequestPlan> {
    let requested = resolve_hosted_tool_model_selector(tool, args);
    let edit = tool == "edit_image";
    let format = string(args, "output_format")
        .or_else(|| string(args, "outputFormat"))
        .map(|value| value.trim().to_lowercase());
    let gif = format.as_deref() == Some("gif");
    let gif_default = !edit && requested.is_none() && gif;
    let filter: Option<fn(&str) -> bool> = if requested.as_deref().is_some_and(is_seedream_model) {
        None
    } else if gif_default || requested.as_deref().is_some_and(is_gpt_image_26_model) {
        Some(is_gpt_image_26_model)
    } else if edit {
        Some(|id| is_edit_image_model(id) && !is_seedream_model(id))
    } else {
        Some(|id| !is_seedream_model(id) && !is_gpt_image_26_model(id))
    };
    let preferred: &[&str] = if gif_default {
        &["gpt-image-2.6-flare"]
    } else {
        &[]
    };
    let model = select(
        models,
        "image",
        requested.as_deref(),
        None,
        preferred,
        filter,
    )?;
    let mut plan = ToolRequestPlan::new("image", &model, args, options)?;
    if edit {
        let inputs: Vec<_> = string(args, "source_image_url")
            .into_iter()
            .chain(
                args.get("reference_image_urls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.trim().is_empty()),
            )
            .collect();
        if inputs.is_empty() {
            return Err(Error::InvalidInput(
                "edit_image requires source_image_url or reference_image_urls".into(),
            ));
        }
        let max = crate::projects::max_context_images(&model);
        if inputs.len() > max {
            return Err(Error::InvalidInput(format!(
                "{model} accepts at most {max} reference images; this request supplies {}",
                inputs.len()
            )));
        }
        plan.params
            .insert("contextImages".into(), json!(vec![true; inputs.len()]));
        for (index, input) in inputs.iter().enumerate() {
            plan.asset(
                AssetRole::ContextImage((index + 1) as u8),
                input,
                "image",
                false,
            );
        }
    } else if !crate::utils::is_seedream_model(&model) {
        plan.copy(args, &[("steps", "steps")]);
    }
    if !crate::utils::is_seedream_model(&model) {
        plan.copy(
            args,
            &[("negative_prompt", "negativePrompt"), ("seed", "seed")],
        );
    }
    if gif {
        plan.copy(args, &[("width", "width"), ("height", "height")]);
    } else if args
        .get("width")
        .and_then(Value::as_f64)
        .is_some_and(|n| n != 0.0)
        && args
            .get("height")
            .and_then(Value::as_f64)
            .is_some_and(|n| n != 0.0)
    {
        plan.copy(args, &[("width", "width"), ("height", "height")]);
        plan.params.insert("sizePreset".into(), json!("custom"));
    }
    for (snake, camel) in [
        ("gpt_image_quality", "gptImageQuality"),
        ("gpt_image_background", "gptImageBackground"),
    ] {
        if let Some(value) = args
            .get(snake)
            .filter(|v| !v.is_null())
            .or_else(|| args.get(camel))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            plan.params
                .insert(camel.into(), json!(value.trim().to_lowercase()));
        }
    }
    if let Some(value) = args.get("mask_image_url").filter(|v| !v.is_null()) {
        plan.params.insert("gptImageMaskUrl".into(), value.clone());
    }
    plan.copy(
        args,
        &[
            ("gptImageComplexity", "gptImageComplexity"),
            ("gptImageFrameCount", "gptImageFrameCount"),
            ("gptImageFrameDurationMs", "gptImageFrameDurationMs"),
            ("markup_image_url", "gptImageMarkupUrl"),
        ],
    );
    if let Some(value) = args
        .get("gpt_image_output_compression")
        .filter(|v| !v.is_null())
        .or_else(|| {
            args.get("gptImageOutputCompression")
                .filter(|v| !v.is_null())
        })
    {
        plan.params
            .insert("gptImageOutputCompression".into(), value.clone());
    }
    if let Some(format) = format.as_deref() {
        let format = if format == "jpeg" { "jpg" } else { format };
        if ["png", "jpg", "webp", "gif"].contains(&format) {
            plan.params.insert("outputFormat".into(), json!(format));
        }
    }
    Ok(plan)
}

#[cfg(test)]
#[path = "image_566_tests.rs"]
mod contracts_566_tests;
#[cfg(test)]
#[path = "image_569_tests.rs"]
mod contracts_569_tests;
