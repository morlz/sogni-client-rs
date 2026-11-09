use super::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/upstream-5-79.json")).unwrap()
}

fn request(params: &Value) -> Result<Value> {
    let options = ModelOptions {
        model_id: params["modelId"].as_str().unwrap().to_owned(),
        media_type: params["type"].as_str().unwrap().to_owned(),
        raw: json!({
            "steps": {"min":1,"max":50,"default":1},
            "guidance": {"min":0,"max":20,"default":1},
            "sampler": {"allowed":[],"default":null},
            "scheduler": {"allowed":[],"default":null},
        }),
    };
    build_job_request("UPSTREAM-5-79", params.as_object().unwrap(), &options, None)
}

#[test]
fn generation_579_matches_compiled_typescript_fields_and_refusals() {
    let fixture = fixture();
    assert_eq!(fixture["version"], "5.79.0");
    assert_eq!(
        fixture["commit"],
        "c397d41c04d5219f7a640662f621d629b82a69a6"
    );
    for case in fixture["cases"].as_array().unwrap() {
        let actual =
            request(&case["params"]).unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        for (target, expected) in [
            (&actual, &case["request"]),
            (&actual["keyFrames"][0], &case["frame"]),
        ] {
            for (field, value) in expected.as_object().unwrap() {
                if value.is_null() {
                    assert!(
                        target.get(field).is_none(),
                        "{} unexpectedly sent {field}",
                        case["name"]
                    );
                } else if let Some(number) = value.as_f64() {
                    assert_eq!(
                        target[field].as_f64(),
                        Some(number),
                        "{}: {field}",
                        case["name"]
                    );
                } else {
                    assert_eq!(&target[field], value, "{}: {field}", case["name"]);
                }
            }
        }
    }
    for case in fixture["refusals"].as_array().unwrap() {
        let error = request(&case["params"]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(case["message"].as_str().unwrap()),
            "{}: {error}; expected {}",
            case["name"],
            case["message"]
        );
    }
}

#[test]
fn gif_geometry_matches_upstream_layouts_including_unused_cells() {
    for case in fixture()["sizes"].as_array().unwrap() {
        let count = Some(case["count"].as_u64().unwrap() as u32);
        let (width, height) =
            resolve_gpt_image_gif_size(None, None, count, case["ratio"].as_f64()).unwrap();
        assert_eq!(
            json!([width, height]),
            json!([case["width"], case["height"]]),
            "{case}"
        );
        let sheet = get_gpt_image_gif_sheet(width, height, count).unwrap();
        assert_eq!(
            json!({"columns":sheet.columns, "rows":sheet.rows, "width":sheet.width, "height":sheet.height}),
            case["sheet"],
            "{case}"
        );
    }
    let bounds = get_gpt_image_gif_frame_bounds(Some(2)).unwrap();
    assert_eq!(bounds.max_pixels, 1_036_800);
    assert_eq!(bounds.min_pixels, 81_920);
    assert!(get_gpt_image_gif_sheet(16, 16, Some(9)).is_err());
    assert!(get_gpt_image_gif_sheet(u32::MAX - 15, u32::MAX - 15, None).is_err());
    assert!(resolve_gpt_image_gif_size(None, None, None, Some(f64::NAN)).is_err());
}

#[test]
fn priority_recovery_preserves_an_explicit_false_and_leaves_legacy_records_untouched() {
    for value in [Value::Null, json!(true), json!(false), json!("invalid")] {
        let params = recovered_params(&json!({
            "model":{"id":"flux1-schnell-fp8","type":"image"},"priorityMode":value
        }));
        assert_eq!(
            params.get("priorityMode").and_then(Value::as_bool),
            value.as_bool()
        );
    }
}
