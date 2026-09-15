use super::*;

pub(super) fn plan(
    tool: &str,
    args: &Value,
    options: &Value,
    models: &[Value],
) -> Result<ToolRequestPlan> {
    let requested = resolve_hosted_tool_model_selector(tool, args);
    let edit = tool == "edit_image";
    let model = select(
        models,
        "image",
        requested.as_deref(),
        None,
        &[],
        edit.then_some(is_edit_image_model),
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
        let max = if crate::utils::is_gpt_image_model(&model) {
            16
        } else if model.contains("krea2_identity_edit") {
            2
        } else {
            3
        };
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
    } else {
        plan.copy(args, &[("steps", "steps")]);
    }
    plan.copy(
        args,
        &[("negative_prompt", "negativePrompt"), ("seed", "seed")],
    );
    if args
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
    if let Some(value) = args
        .get("output_format")
        .filter(|v| !v.is_null())
        .or_else(|| args.get("outputFormat"))
        .and_then(Value::as_str)
    {
        let format = value.trim().to_lowercase();
        let format = if format == "jpeg" { "jpg" } else { &format };
        if ["png", "jpg", "webp"].contains(&format) {
            plan.params.insert("outputFormat".into(), json!(format));
        }
    }
    Ok(plan)
}
