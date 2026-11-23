use super::*;

pub(super) fn generation_579_result(request: &Value) -> Option<Value> {
    let frame = &request["keyFrames"][0];
    let mut result = json!({"jobID":request["jobID"],"imgID":"JOB-1","resultUrl":"https://example.test/result.mp4"});
    match frame["modelID"].as_str()? {
        "sam3_video_segment_fp16" => {
            result["sourceVideoSha256"] = json!("a".repeat(64));
            result["sha256"] = json!("b".repeat(64));
            result["maskFrames"] = json!(255);
            result["maskFps"] = json!(30000.0 / 1001.0);
            result["samVersion"] = json!("sam3.1@fixture");
        }
        "seedance-2-5" if frame["seedanceDraft"] == true => {
            result["seedanceDraft"] = json!(true);
            result["seedanceDraftExpiresAt"] = json!("2026-11-17T20:00:00.000Z");
        }
        "seedance-2-5" if frame["seedanceDraftJobId"].is_string() => {
            result["seedanceDraftJobId"] = frame["seedanceDraftJobId"].clone();
            if let Some(resolution) = frame.get("seedanceDraftFinalResolution") {
                result["seedanceDraftFinalResolution"] = resolution.clone();
            }
            result["lastFrameKey"] = json!("final-frame.png");
            result["lastFrameUrl"] = json!("https://example.test/final-frame.png");
        }
        _ => return None,
    }
    Some(result)
}
