use super::*;
use clap::Parser;

fn parse(flags: &[&str]) -> Args {
    let default_model = if flags.contains(&"--model") {
        &[][..]
    } else {
        &["--model", "ace_step_1.5_xl_turbo"][..]
    };
    Args::try_parse_from(
        std::iter::once("x")
            .chain(default_model.iter().copied())
            .chain(flags.iter().copied()),
    )
    .expect("valid CLI syntax")
}

fn request(flags: &[&str]) -> Result<sogni_client::ProjectRequest> {
    super::super::build(&parse(flags))
}

fn error(flags: &[&str]) -> String {
    request(flags).unwrap_err().to_string()
}

#[test]
fn model_tables_match_upstream_defaults_and_choices() {
    let cases = [
        (
            "ace_step_1.5_xl_turbo",
            "ACE-Step 1.5 XL Turbo",
            8,
            (4, 16),
            None,
            "euler",
            XL_SAMPLERS,
            "simple",
            SIMPLE_SCHEDULER,
        ),
        (
            "ace_step_1.5_xl_sft",
            "ACE-Step 1.5 XL SFT",
            50,
            (10, 200),
            Some(7.0),
            "euler",
            XL_SAMPLERS,
            "simple",
            SIMPLE_SCHEDULER,
        ),
        (
            "ace_step_1.5_turbo",
            "ACE-Step 1.5 Turbo (Legacy)",
            8,
            (4, 16),
            None,
            "euler",
            XL_SAMPLERS,
            "simple",
            SIMPLE_SCHEDULER,
        ),
        (
            "ace_step_1.5_sft",
            "ACE-Step 1.5 SFT (Legacy)",
            50,
            (10, 200),
            Some(5.0),
            "er_sde",
            LEGACY_SFT_SAMPLERS,
            "linear_quadratic",
            LEGACY_SFT_SCHEDULERS,
        ),
    ];
    for (id, name, steps, range, guidance, sampler, samplers, scheduler, schedulers) in cases {
        let spec = model_spec(id).unwrap();
        assert_eq!(spec.name, name);
        assert_eq!(spec.steps, steps);
        assert_eq!(spec.step_range, range);
        assert_eq!(spec.guidance, guidance);
        assert_eq!(spec.sampler, sampler);
        assert_eq!(spec.samplers, samplers);
        assert_eq!(spec.scheduler, scheduler);
        assert_eq!(spec.schedulers, schedulers);
    }
}

#[test]
fn key_and_language_tables_match_upstream_sets() {
    assert_eq!(KEYS.len(), 70);
    for key in ["A major", "A# minor", "C♭ major", "F♯ minor", "G♭ minor"] {
        assert!(KEYS.contains(&key));
        assert!(request(&["--keyscale", key]).is_ok(), "{key}");
    }
    assert_eq!(LANGUAGES.len(), 51);
    for language in ["ar", "en", "ru", "yue", "zh", "unknown"] {
        assert!(LANGUAGES.contains(&language));
        assert!(request(&["--language", language]).is_ok(), "{language}");
    }
    assert!(error(&["--keyscale", "H major"]).starts_with("Key/scale must be one of:"));
    assert!(error(&["--language", "xx"]).starts_with("Language must be one of:"));
}

#[test]
fn numeric_boundaries_are_inclusive() {
    for flags in [
        &[
            "--duration",
            "10",
            "--shift",
            "1",
            "--prompt-strength",
            "0",
            "--creativity",
            "0",
            "--batch",
            "1",
        ][..],
        &[
            "--duration",
            "600",
            "--shift",
            "5",
            "--prompt-strength",
            "10",
            "--creativity",
            "2",
            "--batch",
            "512",
        ][..],
        &["--model", "ace_step_1.5_xl_sft", "--guidance", "1"][..],
        &["--model", "ace_step_1.5_sft", "--guidance", "15"][..],
    ] {
        assert!(request(flags).is_ok(), "{flags:?}");
    }
}

#[test]
fn numeric_values_outside_constraints_are_rejected() {
    let cases = [
        (
            &["--duration", "9.99"][..],
            "Duration must be between 10 and 600 seconds",
        ),
        (
            &["--duration", "600.01"][..],
            "Duration must be between 10 and 600 seconds",
        ),
        (&["--bpm", "29"][..], "BPM must be between 30 and 300"),
        (&["--bpm", "301"][..], "BPM must be between 30 and 300"),
        (
            &["--model", "ace_step_1.5_xl_sft", "--guidance", "0.99"][..],
            "Guidance must be between 1 and 15",
        ),
        (
            &["--model", "ace_step_1.5_sft", "--guidance", "15.01"][..],
            "Guidance must be between 1 and 15",
        ),
        (&["--shift", "0.99"][..], "Shift must be between 1 and 5"),
        (&["--shift", "5.01"][..], "Shift must be between 1 and 5"),
        (
            &["--batch", "0"][..],
            "Batch count must be between 1 and 512",
        ),
        (
            &["--batch", "513"][..],
            "Batch count must be between 1 and 512",
        ),
    ];
    for (flags, expected) in cases {
        assert_eq!(error(flags), expected, "{flags:?}");
    }
}

#[test]
fn all_floating_point_inputs_must_be_finite() {
    for (flags, expected) in [
        (&["--duration", "NaN"][..], "Duration must be finite"),
        (
            &["--model", "ace_step_1.5_xl_sft", "--guidance", "inf"][..],
            "Guidance must be finite",
        ),
        (&["--shift", "NaN"][..], "Shift must be finite"),
        (
            &["--prompt-strength", "inf"][..],
            "Prompt strength must be finite",
        ),
        (&["--creativity", "NaN"][..], "Creativity must be finite"),
    ] {
        assert_eq!(error(flags), expected, "{flags:?}");
    }
}

#[test]
fn sampler_and_scheduler_choices_are_model_specific() {
    for (model, samplers, schedulers) in [
        ("ace_step_1.5_xl_turbo", XL_SAMPLERS, SIMPLE_SCHEDULER),
        ("ace_step_1.5_xl_sft", XL_SAMPLERS, SIMPLE_SCHEDULER),
        ("ace_step_1.5_turbo", XL_SAMPLERS, SIMPLE_SCHEDULER),
        (
            "ace_step_1.5_sft",
            LEGACY_SFT_SAMPLERS,
            LEGACY_SFT_SCHEDULERS,
        ),
    ] {
        for sampler in samplers {
            assert!(request(&["--model", model, "--sampler", sampler]).is_ok());
        }
        for scheduler in schedulers {
            assert!(request(&["--model", model, "--scheduler", scheduler]).is_ok());
        }
    }
    assert_eq!(
        error(&["--model", "ace_step_1.5_xl_sft", "--sampler", "er_sde"]),
        "Sampler must be one of: euler, euler_ancestral for ACE-Step 1.5 XL SFT"
    );
    assert_eq!(
        error(&[
            "--model",
            "ace_step_1.5_xl_sft",
            "--scheduler",
            "linear_quadratic"
        ]),
        "Scheduler must be one of: simple for ACE-Step 1.5 XL SFT"
    );
}

#[test]
fn music3_ranges_and_default_settings_are_distinct_from_ace() {
    let spec = model_spec("minimax_music3").unwrap();
    assert!(!spec.ace_controls);
    assert_eq!(spec.step_range, (10, 100));
    assert_eq!(spec.steps, 30);
    assert_eq!(spec.duration_range, (10.0, 300.0));
    assert_eq!(spec.duration_default, 60.0);
    assert_eq!(spec.guidance, Some(1.7));
    assert_eq!(spec.guidance_range, (1.0, 5.0));
    assert_eq!(spec.prompt_strength_default, 1.7);
    for (duration, steps, guidance) in [("10", "10", "1"), ("300", "100", "5")] {
        assert!(
            request(&[
                "--model",
                "minimax_music3",
                "--duration",
                duration,
                "--steps",
                steps,
                "--guidance",
                guidance
            ])
            .is_ok()
        );
    }
    for (key, value, expected) in [
        (
            "--duration",
            "300.01",
            "Duration must be between 10 and 300 seconds for MiniMax Music 3",
        ),
        (
            "--steps",
            "9",
            "Steps must be between 10 and 100 for MiniMax Music 3",
        ),
        (
            "--steps",
            "101",
            "Steps must be between 10 and 100 for MiniMax Music 3",
        ),
        (
            "--guidance",
            "0.99",
            "Guidance must be between 1 and 5 for MiniMax Music 3",
        ),
        (
            "--guidance",
            "5.01",
            "Guidance must be between 1 and 5 for MiniMax Music 3",
        ),
    ] {
        assert_eq!(error(&["--model", "minimax_music3", key, value]), expected);
    }
}

#[test]
fn music3_prompt_directions_accept_values_outside_ace_control_constraints() {
    for (flag, value, direction) in [
        ("--bpm", "20", "Tempo: 20 BPM."),
        ("--keyscale", "D dorian", "Key: D dorian."),
        ("--timesig", "7", "Time signature: 7/4."),
        ("--language", "elvish", "Lyrics language: elvish."),
    ] {
        let request = request(&["--model", "minimax_music3", flag, value, "--dry-run"])
            .expect("Music 3 direction is ordinary prompt text");
        let params = request.params();
        assert_eq!(params["modelId"], "minimax_music3");
        assert!(
            params["positivePrompt"]
                .as_str()
                .unwrap()
                .ends_with(direction),
            "{flag}"
        );
        for key in [
            "bpm",
            "keyscale",
            "timesignature",
            "language",
            "shift",
            "composerMode",
            "creativity",
        ] {
            assert!(
                params.get(key).is_none(),
                "{flag} must not create ACE field {key}"
            );
        }
    }
}

#[test]
fn explicit_ace_still_rejects_out_of_contract_controls() {
    for (flag, value, expected) in [
        ("--bpm", "20", "BPM must be between 30 and 300"),
        ("--keyscale", "D dorian", "Key/scale must be one of:"),
        (
            "--timesig",
            "7",
            "Time signature must be one of: 2, 3, 4, 6",
        ),
        ("--language", "elvish", "Language must be one of:"),
    ] {
        assert!(
            error(&["--model", "ace_step_1.5_xl_turbo", flag, value, "--dry-run"])
                .starts_with(expected),
            "{flag}"
        );
    }
}
