use super::*;

pub(super) const FINALIZE_PROMPT: &str = "Finalize the Seedance 2.5 draft at 1080p.";
pub(super) const FINALIZE_4K_PROMPT: &str = "Finalize the Seedance 2.5 draft at 4K.";

#[derive(Clone, Copy)]
pub(super) enum SeedanceDraftMode<'a> {
    Standard,
    Draft,
    Finalize(&'a str, Option<&'a str>),
}

impl<'a> SeedanceDraftMode<'a> {
    pub(super) fn parse(params: &'a Map<String, Value>, model: &str) -> Result<Self> {
        let draft = params
            .get("seedanceDraft")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Error::InvalidInput("seedanceDraft must be a boolean.".into()))
            })
            .transpose()?
            .unwrap_or(false);
        let draft_id = params
            .get("seedanceDraftJobId")
            .map(|value| {
                value
                    .as_str()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or_else(|| {
                        Error::InvalidInput(
                            "seedanceDraftJobId must be the Sogni job id of a Seedance 2.5 draft."
                                .into(),
                        )
                    })
            })
            .transpose()?;
        let resolution = params
            .get("seedanceDraftFinalResolution")
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| matches!(*value, "1080p" | "4k"))
                    .ok_or_else(|| {
                        Error::InvalidInput(
                            "seedanceDraftFinalResolution must be 1080p or 4k.".into(),
                        )
                    })
            })
            .transpose()?;
        if resolution.is_some() && draft_id.is_none() {
            return Err(Error::InvalidInput(
                "seedanceDraftFinalResolution requires seedanceDraftJobId.".into(),
            ));
        }
        if (draft || draft_id.is_some()) && !is_seedance25_model(model) {
            return Err(Error::InvalidInput(
                "Seedance draft mode is supported only by Seedance 2.5.".into(),
            ));
        }
        match (draft, draft_id) {
            (true, Some(_)) => Err(Error::InvalidInput(
                "seedanceDraft and seedanceDraftJobId are mutually exclusive: create a draft, or finalize one."
                    .into(),
            )),
            (true, None) => Ok(Self::Draft),
            (false, Some(id)) => Ok(Self::Finalize(id, resolution)),
            (false, None) => Ok(Self::Standard),
        }
    }

    pub(super) fn validate_finalization(self, params: &Map<String, Value>) -> Result<()> {
        if !matches!(self, Self::Finalize(..)) {
            return Ok(());
        }
        let mut inherited = Vec::new();
        for field in [
            "referenceImage",
            "referenceImageEnd",
            "referenceImageUrls",
            "referenceVideo",
            "referenceVideoUrls",
            "referenceAudio",
            "referenceAudioUrls",
            "referenceAudioIdentity",
            "referenceMask",
            "referenceFileUrl",
            "referenceLinkUrl",
        ] {
            if params.get(field).is_some_and(|value| {
                value
                    .as_array()
                    .map_or_else(|| truthy_value(value), |items| !items.is_empty())
            }) {
                inherited.push(field);
            }
        }
        for field in [
            "duration",
            "frames",
            "ratio",
            "seed",
            "generateAudio",
            "seedanceTaskType",
        ] {
            if params.contains_key(field) {
                inherited.push(field);
            }
        }
        if !inherited.is_empty() {
            return Err(Error::InvalidInput(format!(
                "A Seedance 2.5 finalize reuses its draft's inputs and settings; remove {}.",
                inherited.join(", ")
            )));
        }
        if params
            .get("numberOfMedia")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            != 1
        {
            return Err(Error::InvalidInput(
                "A Seedance 2.5 finalize renders its draft once; numberOfMedia must be 1.".into(),
            ));
        }
        Ok(())
    }
}

/// Public metadata for a completed Seedance 2.5 draft or final.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeedanceDraftReceipt {
    /// Present as `true` for a draft that can be finalized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seedance_draft: Option<bool>,
    /// Server-supplied ISO 8601 expiry of the draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seedance_draft_expires_at: Option<String>,
    /// Source draft job ID reported by a completed final.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seedance_draft_job_id: Option<String>,
    /// Explicit final output resolution reported by the service; absent values stay absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seedance_draft_final_resolution: Option<String>,
}

impl SeedanceDraftReceipt {
    pub(super) fn from_record(record: &Value) -> Self {
        let mut receipt = Self::default();
        receipt.update_from(record);
        receipt
    }

    pub(super) fn update_from(&mut self, record: &Value) {
        for source in [record.get("result"), Some(record)].into_iter().flatten() {
            if let Some(value) = source
                .get("seedanceDraftFinalResolution")
                .and_then(Value::as_str)
                .filter(|value| matches!(*value, "1080p" | "4k"))
            {
                self.seedance_draft_final_resolution = Some(value.to_owned());
            }
            if source.get("seedanceDraft").and_then(Value::as_bool) == Some(true) {
                self.seedance_draft = Some(true);
            }
            for (field, target) in [
                (
                    "seedanceDraftExpiresAt",
                    &mut self.seedance_draft_expires_at,
                ),
                ("seedanceDraftJobId", &mut self.seedance_draft_job_id),
            ] {
                if let Some(value) = source
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                {
                    *target = Some(value.to_owned());
                }
            }
        }
    }
}
