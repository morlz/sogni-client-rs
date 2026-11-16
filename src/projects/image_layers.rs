use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A transparent PNG layer above the base image of a Seedream result.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageLayer {
    pub url: String,
    pub width: u64,
    pub height: u64,
    pub z_index: u8,
    pub bounding_box: ImageLayerBoundingBox,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_recognition: Option<ImageLayerTextRecognition>,
}

/// Left, top, right and bottom in base-image pixels and on a 0–1000 grid.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ImageLayerBoundingBox {
    pub absolute: [f64; 4],
    pub normalized: [f64; 4],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ImageLayerTextRecognition {
    pub text: String,
    pub lines: Vec<ImageLayerTextLine>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageLayerTextLine {
    pub bounding_box: ImageLayerBoundingBox,
    pub runs: Vec<ImageLayerTextRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ImageLayerTextRun {
    pub text: String,
    pub style: ImageLayerTextStyle,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageLayerTextStyle {
    pub similar_font_family: String,
    pub foreground_color: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub shadow: bool,
    pub translucent: bool,
}

pub(super) fn from_result(record: &Value) -> Option<Vec<ImageLayer>> {
    [record.get("layers"), record.pointer("/result/layers")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .find_map(|raw| {
            let mut layers: Vec<_> = raw.iter().filter_map(normalize).collect();
            layers.sort_by_key(|layer| layer.z_index);
            (!layers.is_empty()).then_some(layers)
        })
}

fn normalize(raw: &Value) -> Option<ImageLayer> {
    let integer = |field: &str, max: u64| {
        raw.get(field)?
            .as_f64()
            .filter(|n| n.fract() == 0.0 && (1.0..=max as f64).contains(n))
            .map(|n| n as u64)
    };
    let text = |field: &str| raw.get(field)?.as_str().map(ToOwned::to_owned);
    Some(ImageLayer {
        url: text("url").filter(|url| !url.is_empty())?,
        width: integer("width", 9_007_199_254_740_991)?,
        height: integer("height", 9_007_199_254_740_991)?,
        z_index: integer("zIndex", 99)? as u8,
        bounding_box: serde_json::from_value(raw.get("boundingBox")?.clone()).ok()?,
        name: text("name"),
        description: text("description"),
        text_recognition: raw
            .get("textRecognition")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recovered_layers_keep_stacking_order_and_omit_malformed_recognition() {
        let layer = |index| {
            json!({
                "url":"https://example.test/layer.png","width":64,"height":32,"zIndex":index,
                "boundingBox":{"absolute":[1,2,65,34],"normalized":[4,8,260,136]}
            })
        };
        let mut top = layer(8);
        top["textRecognition"] = json!({"text":"keep","lines":[]});
        top["private"] = json!("not public");
        let mut front = layer(1);
        front["textRecognition"] = json!({"text":"bad","lines":[{"runs":[{"text":"bad"}]}]});
        let recovered = json!({"result":{"layers":[top,layer(0),front,layer(100)]}});
        let layers = from_result(&recovered).unwrap();
        assert_eq!(
            layers.iter().map(|layer| layer.z_index).collect::<Vec<_>>(),
            [1, 8]
        );
        assert!(layers[0].text_recognition.is_none());
        assert_eq!(layers[1].text_recognition.as_ref().unwrap().text, "keep");
        assert!(json!(layers[1]).get("private").is_none());
    }
}
