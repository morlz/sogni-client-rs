use super::*;
use crate::projects::provenance::normalized_hash;

pub(super) fn normalize_world_receipt(params: &Map<String, Value>) -> Result<Option<Value>> {
    let Some(receipt) = params
        .get("worldGenerationReceipt")
        .filter(|v| !sam3::is_falsy(v))
    else {
        return Ok(None);
    };
    // Applications select their recipe; the service decides eligibility and
    // authorization. The SDK only validates the existing receipt wire shape.
    let stage = receipt.get("stage").and_then(Value::as_str);
    let (stage, fields) = match stage {
        Some("target_still") => ("target_still", ["sourceImageSha256", "selectionHash"]),
        Some("transition") => ("transition", ["firstFrameSha256", "lastFrameSha256"]),
        _ => {
            return Err(Error::InvalidInput(
                "worldGenerationReceipt.stage must be target_still or transition.".into(),
            ));
        }
    };
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
