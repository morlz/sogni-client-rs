use crate::{ApiError, Result};
use serde_json::json;

pub(in crate::projects) fn check_image_upload(input: &[u8], slot: &str) -> Result<()> {
    let head = &input[..input.len().min(64)];
    let lower = head.to_ascii_lowercase();
    let markup = lower
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(&lower)
        .trim_ascii_start();
    let reason = if lower.starts_with(b"data:") {
        "its contents are a data: URL"
    } else if markup.strip_prefix(b"<!doctype").is_some_and(|rest| {
        rest.first().is_some_and(u8::is_ascii_whitespace)
            && rest.trim_ascii_start().starts_with(b"html")
    }) || markup.strip_prefix(b"<html").is_some_and(|rest| {
        rest.first()
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>')
    }) {
        "its contents are an HTML page"
    } else if !head.starts_with(b"GIF8")
        && head.get(..16).is_some_and(|head| {
            head.iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/'))
        })
    {
        "its contents are base64 text"
    } else {
        return Ok(());
    };
    Err(ApiError::new(
        400,
        json!({
            "status":"error","errorCode":0,
            "message":format!("{slot} is not image data: {reason}. Upload the image file itself.")
        }),
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_text_but_keeps_svg_and_binary() {
        for (bytes, description) in [
            (&b"DaTa:image/png;base64,AAAA"[..], "data: URL"),
            (&b"\xef\xbb\xbf \t<!DOCTYPE\tHTML>"[..], "an HTML page"),
            (&b"<HtMl lang=en>"[..], "an HTML page"),
            (&b"iVBORw0KGgoAAAANSUhEUgAA="[..], "base64 text"),
        ] {
            let error = check_image_upload(bytes, "contextImage1").unwrap_err();
            assert!(error.to_string().contains(description));
            assert!(error.to_string().contains("contextImage1"));
        }
        for image in [
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".as_slice(),
            b"GIF89aABCDEFGHIJKL".as_slice(),
            b"\x89PNG\r\n\x1a\n".as_slice(),
        ] {
            check_image_upload(image, "referenceImage").unwrap();
        }
    }
}
