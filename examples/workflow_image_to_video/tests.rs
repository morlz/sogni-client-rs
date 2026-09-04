use super::*;

#[test]
fn transition_sets_lora_and_marker() {
    let args = Args::try_parse_from([
        "x",
        "--model",
        "ltx25-22b-int8_i2v_distilled",
        "--end-image",
        "end.png",
        "--transition",
    ])
    .unwrap();
    let params = build(&args).unwrap().params();
    assert_eq!(params["loras"][0], "transition");
    assert!(
        params["positivePrompt"]
            .as_str()
            .unwrap()
            .contains("zhuanchang")
    );
}

#[test]
fn uncensored_checkpoint_requires_explicit_filter_opt_out() {
    let args =
        Args::try_parse_from(["x", "--model", "ltx23-22b-10eros-v1.4-fp8mixed_i2v"]).unwrap();
    assert!(build(&args).is_err());
}

#[test]
fn ltx_fps_boundaries_and_dry_run_are_validated() {
    let model = "ltx25-22b-int8_i2v_distilled";
    for fps in [1.0, 60.0] {
        assert!(validate_ltx_fps(model, fps).is_ok());
    }
    for fps in [0.0, 61.0, f64::NAN, f64::INFINITY] {
        assert!(validate_ltx_fps(model, fps).is_err());
    }
    assert!(validate_ltx_fps("seedance2", 100.0).is_ok());

    let args = Args::try_parse_from(["x", "--model", model, "--fps", "100", "--dry-run"]).unwrap();
    assert!(
        build(&args)
            .unwrap_err()
            .to_string()
            .contains("between 1 and 60")
    );
}
