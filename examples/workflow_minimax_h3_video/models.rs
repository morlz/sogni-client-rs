use anyhow::{Result, bail};

use super::config::{Args, Mode};

#[derive(Clone)]
pub struct Spec {
    pub id: String,
    pub steps: u32,
    pub sampler: Option<&'static str>,
    pub width: u32,
    pub height: u32,
    pub max_pixels: u64,
}

pub fn resolved_mode(args: &Args) -> Result<Mode> {
    // A model selector carries a workflow mode. Infer it when --mode is absent,
    // but reject a contradictory explicit mode before any media is touched.
    let model_mode = args.model.as_deref().map(infer_mode).transpose()?;
    match (args.mode, model_mode) {
        (Some(requested), Some(actual)) if requested != actual => bail!(
            "model '{}' is a {} model, not {}",
            args.model.as_deref().expect("model is present"),
            actual.as_str(),
            requested.as_str()
        ),
        (Some(requested), _) => Ok(requested),
        (None, Some(actual)) => Ok(actual),
        (None, None) => Ok(Mode::T2v),
    }
}

pub fn spec(args: &Args) -> Result<Spec> {
    let model_mode = resolved_mode(args)?;
    let key = args
        .model
        .clone()
        .unwrap_or_else(|| format!("minimax-h3-{}-balanced", model_mode.as_str()));
    // Friendly selectors are expanded to canonical FL2VA, Ref2VA, or FastVideo
    // ids; already-canonical ids pass through after their mode is checked.
    let direct = key.contains("-fl2va-") || key.contains("-ref2va-") || key.contains("-fastvideo-");
    let fast_h3 = key.contains("-fasth3-") || key.contains("-fastvideo-");
    if fast_h3 && model_mode == Mode::R2v {
        bail!("FastH3 has no R2V mode");
    }
    let turbo = key.ends_with("-turbo") || key.ends_with("_turbo");
    let balanced = key.ends_with("-balanced") || key.ends_with("_balanced");
    validate_alias(&key, model_mode, direct, fast_h3, turbo, balanced)?;
    let id = if direct {
        key
    } else if fast_h3 {
        format!("minimax-h3-fastvideo-int8_{}_turbo", model_mode.as_str())
    } else {
        let checkpoint = if model_mode == Mode::R2v {
            "ref2va"
        } else {
            "fl2va"
        };
        let suffix = if turbo {
            "_turbo"
        } else if balanced {
            "_balanced"
        } else {
            ""
        };
        format!(
            "minimax-h3-{checkpoint}-fp8_{}{suffix}",
            model_mode.as_str()
        )
    };
    let reference_turbo = model_mode == Mode::R2v && turbo;
    Ok(Spec {
        id,
        steps: if turbo {
            4
        } else if balanced {
            8
        } else {
            20
        },
        // LightX2V FL2VA Turbo intentionally leaves sampler selection to the
        // compatible recipe; FastH3 and Ref2VA Turbo are fixed to Euler.
        sampler: if turbo && !fast_h3 && model_mode != Mode::R2v {
            None
        } else if turbo || balanced {
            Some("euler")
        } else {
            Some("res_multistep")
        },
        width: if reference_turbo { 960 } else { 1344 },
        height: if reference_turbo { 544 } else { 768 },
        max_pixels: if reference_turbo { 522_240 } else { 1_032_192 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn mode_is_optional_and_defaults_to_t2v_without_a_model() {
        let args = Args::try_parse_from(["x"]).expect("default arguments");
        assert_eq!(args.mode, None);
        assert_eq!(resolved_mode(&args).unwrap(), Mode::T2v);
        assert_eq!(spec(&args).unwrap().id, "minimax-h3-fl2va-fp8_t2v_balanced");
    }

    #[test]
    fn explicit_model_infers_every_h3_workflow_family() {
        for (key, mode, id) in [
            (
                "minimax-h3-t2v-balanced",
                Mode::T2v,
                "minimax-h3-fl2va-fp8_t2v_balanced",
            ),
            (
                "minimax-h3-i2v-balanced",
                Mode::I2v,
                "minimax-h3-fl2va-fp8_i2v_balanced",
            ),
            (
                "minimax-h3-flf2v-balanced",
                Mode::Flf2v,
                "minimax-h3-fl2va-fp8_flf2v_balanced",
            ),
            (
                "minimax-h3-r2v-balanced",
                Mode::R2v,
                "minimax-h3-ref2va-fp8_r2v_balanced",
            ),
        ] {
            let args = Args::try_parse_from(["x", "--model", key]).expect("model arguments");
            assert_eq!(args.mode, None, "{key}");
            assert_eq!(resolved_mode(&args).unwrap(), mode, "{key}");
            assert_eq!(spec(&args).unwrap().id, id, "{key}");
        }
    }

    #[test]
    fn explicit_mode_and_model_conflict_is_clear() {
        let args =
            Args::try_parse_from(["x", "--mode", "t2v", "--model", "minimax-h3-i2v-balanced"])
                .expect("conflicting arguments still parse");
        assert_eq!(
            resolved_mode(&args).unwrap_err().to_string(),
            "model 'minimax-h3-i2v-balanced' is a i2v model, not t2v"
        );
    }
}

fn infer_mode(key: &str) -> Result<Mode> {
    if key.contains("r2v") {
        Ok(Mode::R2v)
    } else if key.contains("flf2v") {
        Ok(Mode::Flf2v)
    } else if key.contains("i2v") {
        Ok(Mode::I2v)
    } else if key.contains("t2v") {
        Ok(Mode::T2v)
    } else {
        bail!("unknown MiniMax H3 model key: {key}")
    }
}

fn validate_alias(
    key: &str,
    mode: Mode,
    direct: bool,
    fast_h3: bool,
    turbo: bool,
    balanced: bool,
) -> Result<()> {
    if direct {
        if !key.starts_with("minimax-h3-") {
            bail!("unknown MiniMax H3 model id: {key}");
        }
        return Ok(());
    }
    let expected = if fast_h3 {
        format!("minimax-h3-fasth3-{}-turbo", mode.as_str())
    } else {
        format!(
            "minimax-h3-{}{}",
            mode.as_str(),
            if turbo {
                "-turbo"
            } else if balanced {
                "-balanced"
            } else {
                ""
            }
        )
    };
    if key != expected {
        bail!("unknown MiniMax H3 model key: {key}");
    }
    Ok(())
}
