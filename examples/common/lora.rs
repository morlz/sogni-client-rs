use std::collections::HashSet;

use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub struct LoraSetting {
    pub id: String,
    pub strength: f64,
}

/// Parse an ordered `id:strength` list while preserving positional stack semantics.
///
/// Duplicate ids, non-finite strengths, empty stacks, and stacks over eight are
/// rejected before a paid request can be submitted.
pub fn parse_stack(specification: &str, normalize_krea_prefix: bool) -> Result<Vec<LoraSetting>> {
    let mut seen = HashSet::new();
    let mut settings = Vec::new();
    for raw in specification
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (id, strength) = raw.rsplit_once(':').unwrap_or((raw, "1"));
        let id = id.trim();
        if id.is_empty() {
            bail!("missing LoRA id in {raw:?}");
        }
        let id = if normalize_krea_prefix && !id.starts_with("krea2-") {
            format!("krea2-{id}")
        } else {
            id.to_owned()
        };
        let strength = strength
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("invalid LoRA strength in {raw:?}"))?;
        if !strength.is_finite() {
            bail!("LoRA strength must be finite in {raw:?}");
        }
        if !seen.insert(id.clone()) {
            bail!("LoRA {id:?} is listed more than once");
        }
        settings.push(LoraSetting { id, strength });
    }
    if settings.is_empty() {
        bail!("at least one LoRA is required");
    }
    if settings.len() > 8 {
        bail!("at most 8 LoRAs may be stacked in one request");
    }
    Ok(settings)
}

pub fn describe(settings: &[LoraSetting]) -> String {
    settings
        .iter()
        .map(|setting| format!("{}@{:+}", setting.id, setting.strength))
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// Return every ordering of a small stack; intended for controlled LoRA comparisons.
pub fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut output = Vec::new();
    for index in 0..items.len() {
        let mut remaining = items.to_vec();
        let item = remaining.remove(index);
        for mut rest in permutations(&remaining) {
            let mut permutation = vec![item.clone()];
            permutation.append(&mut rest);
            output.push(permutation);
        }
    }
    output
}
