use super::*;
use crate::projects::api::TimedValue;
pub(super) fn validate_option(
    selected: Option<&Value>,
    options: Option<&Value>,
    name: &str,
) -> Result<Option<Value>> {
    let Some(selected) = selected.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let allowed = options
        .and_then(|value| value.get("allowed"))
        .and_then(Value::as_array);
    // A model with no choices has no sampler/scheduler input. In particular,
    // speech tiers omit these entirely; do not forward a meaningless value.
    if allowed.is_none_or(Vec::is_empty) || selected.as_str() == Some("") {
        return Ok(None);
    }
    if let Some(allowed) = allowed {
        if !allowed.is_empty() && !allowed.contains(selected) {
            return Err(Error::InvalidInput(format!(
                "invalid {name} {}; allowed values are {}",
                scalar_string(selected),
                allowed
                    .iter()
                    .map(scalar_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(Some(selected.clone()))
}

pub(super) fn ranged_number(
    value: Option<&Value>,
    name: &str,
    minimum: f64,
    maximum: f64,
) -> Result<f64> {
    let number = number(value)
        .ok_or_else(|| Error::InvalidInput(format!("{name} must be a finite number")))?;
    if number < minimum || number > maximum {
        return Err(Error::InvalidInput(format!(
            "{name} must be between {minimum} and {maximum}"
        )));
    }
    Ok(number)
}

pub(super) fn map_model_options(tier: &Value, media_type: &str) -> Value {
    let mut output = Map::new();
    output.insert("type".into(), json!(media_type));
    for field in ["isUpscale", "requiresContextImage"] {
        if let Some(capability) = tier.get(field).and_then(Value::as_bool) {
            output.insert(field.into(), json!(capability));
        }
    }
    for (name, aliases) in [
        ("sampler", sampler_aliases()),
        ("scheduler", scheduler_aliases()),
    ] {
        let source = if name == "sampler" {
            tier.get("comfySampler").or_else(|| tier.get("sampler"))
        } else {
            tier.get("comfyScheduler").or_else(|| tier.get("scheduler"))
        };
        if media_type != "audio" || source.is_some_and(|value| !value.is_null()) {
            output.insert(name.into(), map_options(source, &aliases));
        }
    }
    for field in [
        "steps",
        "guidance",
        "width",
        "height",
        "duration",
        "bpm",
        "promptStrength",
        "creativity",
        "shift",
    ] {
        if let Some(value) = tier.get(field) {
            output.insert(field.into(), map_range(value));
        }
    }
    for field in [
        "fps",
        "timesignature",
        "language",
        "keyscale",
        "vae",
        "speaker",
    ] {
        if let Some(value) = tier.get(field) {
            // Video FPS can be a numeric range (including fractional rates),
            // whereas image/audio options are enumerations.
            let mapped = if field == "fps" && media_type == "video" {
                value.clone()
            } else {
                map_options(Some(value), &BTreeMap::new())
            };
            output.insert(field.into(), mapped);
        }
    }
    for field in [
        "task",
        "outputResolutions",
        "preservesSourceTiming",
        "requiresReferenceVideo",
    ] {
        if media_type == "video" {
            if let Some(value) = tier.get(field) {
                output.insert(field.into(), value.clone());
            }
        }
    }
    if media_type == "audio" {
        if let Some(value) = tier.get("instruct") {
            output.insert("instruct".into(), json!({"maxLength": value.get("maxLength"), "required": value.get("required") == Some(&json!(true))}));
        }
        if let Some(value) = tier.get("referenceText") {
            output.insert(
                "referenceText".into(),
                json!({"maxLength": value.get("maxLength")}),
            );
        }
        if tier.get("acceptInputAudio") == Some(&json!(true))
            || tier.get("requiresReferenceAudio") == Some(&json!(true))
        {
            output.insert("acceptsReferenceAudio".into(), json!(true));
        }
        if tier.get("requiresReferenceAudio") == Some(&json!(true)) {
            output.insert("requiresReferenceAudio".into(), json!(true));
        }
    }
    if let Some(value) = tier.pointer("/composerMode/default") {
        output.insert("composerMode".into(), json!({"default": value}));
    }
    if let Some(value) = tier.get("maxPixels") {
        output.insert("maxPixels".into(), value.clone());
    }
    Value::Object(output)
}

fn map_options(data: Option<&Value>, aliases: &BTreeMap<&str, &str>) -> Value {
    let allowed = data
        .and_then(|data| data.get("allowed"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().map_or_else(
                        || value.clone(),
                        |value| json!(aliases.get(value).copied().unwrap_or(value)),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let default = data
        .and_then(|data| data.get("default"))
        .cloned()
        .unwrap_or(Value::Null);
    let default = default.as_str().map_or(default.clone(), |value| {
        json!(aliases.get(value).copied().unwrap_or(value))
    });
    json!({"allowed": allowed, "default": default})
}

fn map_range(data: &Value) -> Value {
    let decimals = data.get("decimals").and_then(Value::as_i64).unwrap_or(0);
    let step = if decimals > 0 {
        10_f64.powi(-(decimals as i32))
    } else {
        number(data.get("step")).unwrap_or(1.0)
    };
    json!({
        "min": data.get("min"),
        "max": data.get("max"),
        "step": step,
        "default": data.get("default"),
    })
}

fn sampler_aliases() -> BTreeMap<&'static str, &'static str> {
    [
        ("Euler", "euler"),
        ("Euler a", "euler_a"),
        ("Euler Ancestral", "euler_ancestral"),
        ("Heun", "heun"),
        ("DPM++ 2M", "dpmpp_2m"),
        ("DPM++ 2M SDE", "dpmpp_2m_sde"),
        ("DPM++ SDE", "dpmpp_sde"),
        ("DPM++ 3M SDE", "dpmpp_3m_sde"),
        ("UniPC", "uni_pc"),
        ("LCM (Latent Consistency Model)", "lcm"),
    ]
    .into_iter()
    .collect()
}

fn scheduler_aliases() -> BTreeMap<&'static str, &'static str> {
    [
        ("Simple", "simple"),
        ("Normal", "normal"),
        ("Karras", "karras"),
        ("Exponential", "exponential"),
        ("SGM Uniform", "sgm_uniform"),
        ("DDIM Uniform", "ddim_uniform"),
        ("Beta", "beta"),
        ("Linear Quadratic", "linear_quadratic"),
        ("KL Optimal", "kl_optimal"),
        ("DDIM", "ddim"),
        ("Leading", "leading"),
        ("Linear", "linear"),
    ]
    .into_iter()
    .collect()
}

pub(super) fn parse_cost(response: Value) -> Result<CostEstimate> {
    let project = response
        .pointer("/quote/project")
        .ok_or_else(|| Error::Protocol("estimate response missing quote.project".into()))?;
    Ok(CostEstimate {
        token: project.get("costInToken").cloned().unwrap_or(Value::Null),
        usd: project.get("costInUSD").cloned().unwrap_or(Value::Null),
        spark: project.get("costInSpark").cloned().unwrap_or(Value::Null),
        sogni: project.get("costInSogni").cloned().unwrap_or(Value::Null),
        estimated_render_seconds: response
            .pointer("/benchmark/estimatedRenderTimeSec")
            .and_then(Value::as_f64),
        estimated_total_seconds: response
            .pointer("/benchmark/estimatedTotalTimeSec")
            .and_then(Value::as_f64),
        raw: response,
    })
}

pub(super) fn parse_presigned_post(response: &Value) -> Result<PresignedPost> {
    let data = response.get("data").unwrap_or(response);
    let form = data
        .get("upload")
        .filter(|value| value.is_object())
        .unwrap_or(data);
    let url = form
        .get("url")
        .or_else(|| form.get("uploadUrl"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("multipart upload response missing URL".into()))?;
    let fields = form
        .get("fields")
        .and_then(Value::as_object)
        .map(|fields| {
            fields
                .iter()
                .map(|(name, value)| (name.clone(), scalar_string(value)))
                .collect()
        })
        .unwrap_or_default();
    let max_size_bytes = form
        .get("maxSizeBytes")
        .or_else(|| data.get("maxSizeBytes"))
        .and_then(Value::as_u64);
    let public_url = form
        .get("publicUrl")
        .or_else(|| data.get("publicUrl"))
        .and_then(Value::as_str)
        .map(Url::parse)
        .transpose()?;
    Ok(PresignedPost {
        url: Url::parse(url)?,
        fields,
        max_size_bytes,
        public_url,
    })
}

pub(super) fn response_url(response: Value, field: &str) -> Result<Url> {
    let value = response
        .get("data")
        .and_then(|data| data.get(field))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol(format!("response missing data.{field}")))?;
    Ok(Url::parse(value)?)
}

pub(super) fn fresh_cache(cache: &RwLock<Option<TimedValue>>) -> Option<Value> {
    cache
        .read()
        .as_ref()
        .filter(|cached| cached.loaded_at.elapsed() < MODEL_CACHE_TTL)
        .map(|cached| cached.value.clone())
}

pub(super) fn cached_model_media(
    cache: &RwLock<Option<TimedValue>>,
    model_id: &str,
) -> Option<String> {
    cache.read().as_ref().and_then(|cached| {
        value_array(&cached.value)
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(model_id))
            .and_then(|model| model.get("media"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

pub(super) fn value_array(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .or_else(|| value.get("models").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
