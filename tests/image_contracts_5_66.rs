//! Public routing and schema contracts introduced by TypeScript SDK 5.66.0.
use serde_json::json;
use sogni_client::{HostedTools, is_edit_image_model, resolve_hosted_tool_model_selector};

#[test]
fn image_tool_models_and_aliases_preserve_generate_and_edit_capabilities() {
    let flux = "flux3_image_klein_sd_int8_convrot";
    for tool in ["generate_image", "edit_image"] {
        for alias in [
            "flux-3-klein",
            "flux3-klein",
            "flux-3-image-klein",
            "flux3-image-klein",
            " FLUX 3 IMAGE KLEIN ",
        ] {
            assert_eq!(
                resolve_hosted_tool_model_selector(tool, &json!({"model":alias})).as_deref(),
                Some(flux)
            );
        }
        let schema = HostedTools.get(tool).unwrap();
        assert!(
            schema["function"]["parameters"]["properties"]["model"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("flux-3-klein"))
        );
    }
    assert!(is_edit_image_model(flux));
    let generate = HostedTools.get("generate_image").unwrap();
    let edit = HostedTools.get("edit_image").unwrap();
    for (alias, model) in [
        ("moss-image", "moss_image_sft_int8_convrot"),
        ("moss-image-8step", "moss_image_sft_dmd8_int8_convrot"),
    ] {
        assert_eq!(
            resolve_hosted_tool_model_selector("generate_image", &json!({"model":alias}))
                .as_deref(),
            Some(model)
        );
        assert!(!is_edit_image_model(model));
        assert!(
            generate["function"]["parameters"]["properties"]["model"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(alias))
        );
        assert!(
            !edit["function"]["parameters"]["properties"]["model"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!(alias))
        );
    }
    for schema in [generate, edit] {
        let properties = schema["function"]["parameters"]["properties"]
            .as_object()
            .unwrap();
        for name in [
            "mossRopeScaleX",
            "mossRopeScaleY",
            "moss_rope_scale_x",
            "moss_rope_scale_y",
        ] {
            assert!(
                !properties.contains_key(name),
                "direct project field {name} is not a hosted tool parameter"
            );
        }
    }
}
