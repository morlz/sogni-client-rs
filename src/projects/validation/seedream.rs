use super::*;
use crate::SEEDREAM_5_5_UNCENSORED_MODEL_ID;

pub(in crate::projects) const SEEDREAM_FIELDS: [&str; 5] = [
    "seedreamBackground",
    "seedreamLayerDecomposition",
    "seedreamLayerSize",
    "seedreamLayerMaxImages",
    "seedreamLayerTextRecognition",
];

pub(in crate::projects) fn validate_seedream_options(
    params: &Map<String, Value>,
    model_id: &str,
) -> Result<()> {
    if !crate::utils::is_seedream_model(model_id) {
        if SEEDREAM_FIELDS[1..]
            .iter()
            .any(|field| params.contains_key(*field))
        {
            return Err(Error::InvalidInput(
                "Seedream layer options require a Seedream model".into(),
            ));
        }
        if params.contains_key("seedreamBackground") {
            return Err(requires_seedream_55("seedreamBackground"));
        }
        return Ok(());
    }
    let version_55 = model_id == SEEDREAM_5_5_UNCENSORED_MODEL_ID;
    if !version_55 {
        for field in [
            "seedreamBackground",
            "seedreamLayerMaxImages",
            "seedreamLayerTextRecognition",
        ] {
            if params.contains_key(field) {
                return Err(requires_seedream_55(field));
            }
        }
    }
    if params.get("contextImages").is_some_and(|value| {
        value.as_array().is_none_or(|images| {
            images.len() > 10 || images.iter().any(|image| !truthy_value(image))
        })
    }) {
        return Err(Error::InvalidInput(format!(
            "{model_id} accepts up to 10 non-empty reference images in source order"
        )));
    }
    validate_layers(params)?;
    let custom = params.get("sizePreset").is_none()
        || params.get("sizePreset").and_then(Value::as_str) == Some("custom");
    if !custom || !truthy(params.get("width")) || !truthy(params.get("height")) {
        return Ok(());
    }
    let width = number(params.get("width")).unwrap_or(f64::NAN);
    let height = number(params.get("height")).unwrap_or(f64::NAN);
    let pixels = width * height;
    let (maximum, label) = if version_55 {
        (4_624_220.0, "4,624,220")
    } else {
        (4_194_304.0, "4,194,304")
    };
    if !(921_600.0..=maximum).contains(&pixels) || width.max(height) / width.min(height) > 16.0 {
        return Err(Error::InvalidInput(format!(
            "{model_id} renders 921,600 to {label} pixels at an aspect ratio from 1:16 to 16:1; got {width}x{height}"
        )));
    }
    Ok(())
}

fn requires_seedream_55(field: &str) -> Error {
    Error::InvalidInput(format!(
        "{field} requires {SEEDREAM_5_5_UNCENSORED_MODEL_ID}"
    ))
}

fn validate_layers(params: &Map<String, Value>) -> Result<()> {
    let invalid = |message: &str| Error::InvalidInput(message.into());
    let decomposition = params
        .get("seedreamLayerDecomposition")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid("seedreamLayerDecomposition must be a boolean"))
        })
        .transpose()?
        .unwrap_or(false);
    for (field, valid, expected) in [
        (
            "seedreamLayerSize",
            params
                .get("seedreamLayerSize")
                .is_none_or(|v| matches!(v.as_str(), Some("1K" | "1.5K" | "2K"))),
            "must be 1K, 1.5K or 2K",
        ),
        (
            "seedreamLayerMaxImages",
            params.get("seedreamLayerMaxImages").is_none_or(|v| {
                v.as_f64()
                    .is_some_and(|n| n.fract() == 0.0 && (2.0..=100.0).contains(&n))
            }),
            "must be an integer from 2 to 100",
        ),
        (
            "seedreamLayerTextRecognition",
            params
                .get("seedreamLayerTextRecognition")
                .is_none_or(Value::is_boolean),
            "must be a boolean",
        ),
    ] {
        if !valid {
            return Err(Error::InvalidInput(format!("{field} {expected}")));
        }
        if params.contains_key(field) && !decomposition {
            return Err(Error::InvalidInput(format!(
                "{field} requires seedreamLayerDecomposition"
            )));
        }
    }
    if decomposition {
        let refs = params.get("contextImages").and_then(Value::as_array);
        let slots = (1..=16)
            .filter(|index| {
                truthy(params.get(&format!("contextImage{index}")))
                    || refs
                        .and_then(|items| items.get(index - 1))
                        .is_some_and(truthy_value)
            })
            .count();
        if slots != 1 {
            return Err(invalid(
                "Seedream layer decomposition splits exactly one reference image",
            ));
        }
        if params
            .get("numberOfMedia")
            .map_or(1.0, |v| v.as_f64().unwrap_or_default())
            != 1.0
        {
            return Err(invalid(
                "Seedream layer decomposition renders one result: set numberOfMedia to 1",
            ));
        }
    }
    if let Some(background) = params.get("seedreamBackground") {
        if !matches!(background.as_str(), Some("opaque" | "transparent" | "auto")) {
            return Err(invalid(
                "seedreamBackground must be opaque, transparent or auto",
            ));
        }
        if decomposition {
            return Err(invalid(
                "seedreamBackground cannot combine with seedreamLayerDecomposition",
            ));
        }
        if background != "opaque"
            && params.get("outputFormat").and_then(Value::as_str) == Some("jpg")
        {
            return Err(invalid(
                "Transparent or auto Seedream background requires PNG or WebP output",
            ));
        }
    }
    Ok(())
}
