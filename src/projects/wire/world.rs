use super::*;
use crate::projects::provenance::normalized_hash;

pub(super) fn normalize_world_receipt(params: &Map<String, Value>) -> Result<Option<Value>> {
    let Some(receipt) = params
        .get("worldGenerationReceipt")
        .filter(|v| !sam3::is_falsy(v))
    else {
        return Ok(None);
    };
    if params.get("appSource").and_then(Value::as_str) != Some("sogni-world") {
        return Err(Error::InvalidInput(
            "worldGenerationReceipt requires appSource \"sogni-world\".".into(),
        ));
    }
    let stage = receipt.get("stage").and_then(Value::as_str);
    let (stage, model_id, fields) = match stage {
        Some("target_still") => (
            "target_still",
            "krea2_identity_edit_sogni_v0_3_alpha",
            ["sourceImageSha256", "selectionHash"],
        ),
        Some("transition") => (
            "transition",
            "minimax-h3-fastvideo-int8_flf2v_turbo",
            ["firstFrameSha256", "lastFrameSha256"],
        ),
        _ => {
            return Err(Error::InvalidInput(
                "worldGenerationReceipt.stage must be target_still or transition.".into(),
            ));
        }
    };
    if params.get("modelId").and_then(Value::as_str) != Some(model_id) {
        return Err(Error::InvalidInput(format!(
            "The {stage} receipt requires {model_id}."
        )));
    }
    let mut normalized = json!({"stage": stage});
    for field in fields {
        let hash = receipt
            .get(field)
            .and_then(normalized_hash)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "worldGenerationReceipt.{field} must be a SHA-256 hex digest."
                ))
            })?;
        normalized[field] = json!(hash);
    }
    Ok(Some(normalized))
}
