use super::*;

#[test]
fn direct_tool_requests_match_fresh_typescript_builders() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../fixtures/upstream-tool-requests.json")).unwrap();
    for case in fixtures["requests"].as_array().unwrap() {
        let plan = plan_tool_request(
            case["tool"].as_str().unwrap(),
            &case["args"],
            &case["options"],
            fixtures["models"].as_array().unwrap(),
        )
        .unwrap();
        // JSON treats integer and floating number representations alike.
        fn normalize(value: &mut Value) {
            match value {
                Value::Number(number) if number.as_f64().is_some_and(|n| n.fract() == 0.0) => {
                    *value = json!(number.as_f64().unwrap() as i64)
                }
                Value::Array(array) => array.iter_mut().for_each(normalize),
                Value::Object(object) => object.values_mut().for_each(normalize),
                _ => {}
            }
        }
        let mut actual = Value::Object(plan.params);
        let mut expected = case["expected"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{}: {}", case["tool"], case["args"]);
    }
}

#[test]
fn h3_audio_modes_and_indexed_references_do_not_fall_back_silently() {
    let models = vec![
        json!({"id":"minimax-h3-fastvideo-int8_flfa2v_turbo","media":"video"}),
        json!({"id":"ltx23-22b-fp8_a2v_distilled","media":"video"}),
    ];
    let args = json!({"videoModel":"minimax-h3-fasth3-flfa2v-turbo","prompt":"a song","reference_audio_url":"data:audio/wav;base64,UklGRgAAAABXQVZF"});
    assert!(
        plan_tool_request("sound_to_video", &args, &Value::Null, &models)
            .unwrap_err()
            .to_string()
            .contains("needs reference_image_url and reference_image_end_url")
    );
    assert!(
        plan_tool_request(
            "generate_video",
            &json!({"referenceImageIndices":[0]}),
            &Value::Null,
            &models
        )
        .unwrap_err()
        .to_string()
        .contains("mediaContext")
    );
    for tool in ["generate_speech", "upscale_image", "upscale_video"] {
        assert!(
            plan_tool_request(tool, &json!({}), &Value::Null, &models)
                .unwrap_err()
                .to_string()
                .contains("hosted chat")
        );
    }
}

fn music_pool() -> Vec<Value> {
    vec![
        json!({"id":"ace_step_1.5_xl_sft","media":"audio","workerCount":40}),
        json!({"id":"ace_step_1.5_xl_turbo","media":"audio","workerCount":30}),
        json!({"id":"minimax_music3","media":"audio","workerCount":1}),
        json!({"id":"qwen3_tts_1.7b_custom_voice_bf16","media":"audio","workerCount":50}),
    ]
}

#[test]
fn music_prefers_music3_without_ace_controls_even_when_speech_has_more_workers() {
    let args = json!({
        "prompt":"Warm lo-fi hip hop at 84 BPM in A minor", "duration":300,
        "bpm":84,"keyscale":"A minor","timesignature":4,"language":"en",
        "composer_mode":false,"creativity":0.7,"prompt_strength":0,
        "lyrics":"[Chorus]\nA song", "output_format":"wav","seed":0,
    });
    let plan = plan_tool_request("generate_music", &args, &Value::Null, &music_pool()).unwrap();
    assert_eq!(plan.params["modelId"], "minimax_music3");
    for key in [
        "bpm",
        "keyscale",
        "timesignature",
        "language",
        "composerMode",
        "creativity",
    ] {
        assert!(!plan.params.contains_key(key), "{key}");
    }
    assert_eq!(plan.params["positivePrompt"], args["prompt"]);
    assert_eq!(plan.params["lyrics"], args["lyrics"]);
    assert_eq!(plan.params["duration"], 300);
    assert_eq!(plan.params["outputFormat"], "wav");
    assert_eq!(plan.params["promptStrength"], 0.0);
    assert_eq!(plan.params["seed"], 0);
    let music = crate::chat::HostedTools.get("generate_music").unwrap();
    let properties = &music["function"]["parameters"]["properties"];
    assert_eq!(properties["model"]["enum"][0], "minimax_music3");
    assert!(
        properties["duration"]["description"]
            .as_str()
            .unwrap()
            .starts_with("Duration in seconds. music3 (the default model): 10-300, default 60")
    );
    for key in ["bpm", "keyscale", "timesig"] {
        assert!(
            properties[key]["description"]
                .as_str()
                .unwrap()
                .starts_with("ACE-Step (turbo, sft) only")
        );
    }
}

#[test]
fn music_long_tracks_fall_back_to_ace_but_explicit_music3_stays_explicit() {
    let models = music_pool();
    for duration in [300.001, 420.0] {
        let plan = plan_tool_request(
            "generate_music",
            &json!({"duration":duration}),
            &Value::Null,
            &models,
        )
        .unwrap();
        assert_eq!(plan.params["modelId"], "ace_step_1.5_xl_turbo");
        assert_eq!(plan.params["duration"], json!(duration));
    }
    let args = json!({"model":"minimax_music3","duration":420,"bpm":84,"language":"en"});
    let explicit = plan_tool_request("generate_music", &args, &Value::Null, &models).unwrap();
    assert_eq!(explicit.params["modelId"], "minimax_music3");
    assert_eq!(explicit.params["duration"], 420);
    assert!(!explicit.params.contains_key("bpm"));
    assert!(!explicit.params.contains_key("language"));
    let fallback = models
        .into_iter()
        .filter(|model| model["id"] != "minimax_music3")
        .collect::<Vec<_>>();
    let plan = plan_tool_request("generate_music", &json!({}), &Value::Null, &fallback).unwrap();
    assert_eq!(plan.params["modelId"], "ace_step_1.5_xl_turbo");
}

#[test]
fn explicit_ace_music_retains_controls_and_speech_only_catalogs_are_rejected() {
    let args = json!({
        "model":"ace_step_1.5_xl_sft", "duration":420,"bpm":84,"keyscale":"A minor",
        "timesignature":4,"language":"en","composer_mode":false,"creativity":0.7,
    });
    let plan = plan_tool_request("generate_music", &args, &Value::Null, &music_pool()).unwrap();
    assert_eq!(plan.params["modelId"], "ace_step_1.5_xl_sft");
    assert_eq!(plan.params["bpm"], 84);
    assert_eq!(plan.params["keyscale"], "A minor");
    assert_eq!(plan.params["timesignature"], "4");
    assert_eq!(plan.params["language"], "en");
    assert_eq!(plan.params["composerMode"], false);
    assert_eq!(plan.params["creativity"], 0.7);
    for args in [
        json!({}),
        json!({"duration":420}),
        json!({"model":"qwen3_tts_1.7b_custom_voice_bf16"}),
    ] {
        let error = plan_tool_request(
            "generate_music",
            &args,
            &Value::Null,
            &[json!({"id":"qwen3_tts_1.7b_custom_voice_bf16","media":"audio","workerCount":50})],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("No compatible audio models currently available")
        );
    }
}
