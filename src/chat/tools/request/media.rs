use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;

use super::*;
use crate::{MediaSource, RestClient};

#[derive(Debug)]
pub(in crate::chat::tools) struct ToolMedia {
    pub role: AssetRole,
    pub input: String,
    pub media_type: &'static str,
    pub remote: bool,
}

impl ToolMedia {
    pub async fn source(&self, rest: &RestClient) -> Result<MediaSource> {
        let limit = byte_limit(self.media_type);
        let (bytes, mime) =
            if let Some(url) = self.remote.then(|| trusted_url(&self.input)).flatten() {
                let bytes = rest.get_tool_media(url, limit).await?;
                let mime = detected_mime(self.media_type, &bytes).ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "Remote {} input is not a supported format",
                        self.media_type
                    ))
                })?;
                (bytes, mime.into())
            } else {
                parse_inline(&self.input, self.media_type)?
            };
        validate_bytes(self.media_type, &mime, &bytes)?;
        Ok(MediaSource::named_bytes(bytes, "tool-input", mime))
    }
}

fn byte_limit(media: &str) -> usize {
    (match media {
        "image" => 20,
        "audio" => 50,
        _ => 100,
    }) * 1024
        * 1024
}

fn trusted_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value.trim()).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let s3 =
        regex::Regex::new(r"^(?:[a-z0-9.-]+\.s3(?:\.[a-z0-9-]+)?|s3\.[a-z0-9-]+)\.amazonaws\.com$")
            .unwrap();
    (host == "cdn.sogni.ai"
        || host.ends_with(".sogni.ai")
        || host.ends_with(".cloudfront.net")
        || s3.is_match(host))
    .then_some(url)
}

fn parse_inline(input: &str, media: &str) -> Result<(Bytes, String)> {
    let error = || {
        Error::InvalidInput(format!(
            "Only inline base64-encoded data URIs are supported for {media} inputs; remote URLs are not allowed"
        ))
    };
    let (header, body) = input.trim().split_once(',').ok_or_else(error)?;
    let header = header.to_ascii_lowercase();
    let mime = header
        .strip_prefix("data:")
        .and_then(|h| h.strip_suffix(";base64"))
        .filter(|mime| !mime.is_empty() && !mime.contains([';', ',']))
        .ok_or_else(error)?;
    let mut body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if body.len().saturating_mul(3) / 4 > byte_limit(media) + 2 {
        return Err(Error::InvalidInput(format!(
            "{media} input exceeds {}MB limit",
            byte_limit(media) / 1024 / 1024
        )));
    }
    if body.len() % 4 == 1 {
        return Err(Error::InvalidInput("Invalid base64 payload".into()));
    }
    while body.len() % 4 != 0 {
        body.push('=');
    }
    let bytes = STANDARD
        .decode(&body)
        .map_err(|_| Error::InvalidInput("Invalid base64 payload".into()))?;
    if bytes.is_empty() || STANDARD.encode(&bytes) != body {
        return Err(Error::InvalidInput("Invalid base64 payload".into()));
    }
    if bytes.len() > byte_limit(media) {
        return Err(Error::InvalidInput(format!(
            "{media} input exceeds {}MB limit",
            byte_limit(media) / 1024 / 1024
        )));
    }
    validate_bytes(media, mime, &bytes)?;
    Ok((Bytes::from(bytes), mime.into()))
}

fn detected_mime(media: &str, bytes: &[u8]) -> Option<&'static str> {
    match media {
        "image" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => Some("image/png"),
        "image" if bytes.starts_with(b"\xff\xd8\xff") => Some("image/jpeg"),
        "audio" if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") => {
            Some("audio/wav")
        }
        "audio"
            if bytes.starts_with(b"ID3")
                || bytes.len() >= 2
                    && bytes[0] == 0xff
                    && bytes[1] & 0xe0 == 0xe0
                    && (bytes[1] >> 3) & 3 != 1
                    && (bytes[1] >> 1) & 3 != 0 =>
        {
            Some("audio/mpeg")
        }
        "audio"
            if bytes.len() >= 12
                && bytes.get(4..8) == Some(b"ftyp")
                && bytes.get(8..12) != Some(b"qt  ") =>
        {
            Some("audio/mp4")
        }
        "video" if bytes.len() >= 12 && bytes.get(4..8) == Some(b"ftyp") => {
            Some(if bytes.get(8..12) == Some(b"qt  ") {
                "video/quicktime"
            } else {
                "video/mp4"
            })
        }
        _ => None,
    }
}

fn validate_bytes(media: &str, mime: &str, bytes: &[u8]) -> Result<()> {
    let canonical = match mime {
        "image/jpg" => "image/jpeg",
        "audio/mp3" => "audio/mpeg",
        "audio/wave" | "audio/x-wav" => "audio/wav",
        "audio/m4a" | "audio/x-m4a" => "audio/mp4",
        other => other,
    };
    if detected_mime(media, bytes) != Some(canonical) {
        return Err(Error::InvalidInput(format!(
            "Inline {media} data does not match declared MIME type {mime}"
        )));
    }
    if media == "image" && !has_image_dimensions(bytes, canonical) {
        return Err(Error::InvalidInput(
            "Unable to determine inline image dimensions".into(),
        ));
    }
    Ok(())
}

fn has_image_dimensions(bytes: &[u8], mime: &str) -> bool {
    if mime == "image/png" {
        return bytes.get(12..16) == Some(b"IHDR")
            && bytes
                .get(16..24)
                .is_some_and(|size| size[..4] != [0; 4] && size[4..] != [0; 4]);
    }
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return false;
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let Some(&marker) = bytes.get(offset) else {
            return false;
        };
        offset += 1;
        if matches!(marker, 0xd8 | 0xd9 | 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let Some(length) = bytes.get(offset..offset + 2) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
        if length < 2 || offset + length > bytes.len() {
            return false;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            return length >= 7
                && bytes[offset + 3..offset + 5] != [0; 2]
                && bytes[offset + 5..offset + 7] != [0; 2];
        }
        offset += length;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_media_rejects_wrong_types_bad_base64_and_remote_urls() {
        for input in [
            "https://example.com/a.png",
            "data:image/png;base64,AAAA",
            "data:image/png;base64,A===",
            "data:text/plain;base64,aGVsbG8=",
        ] {
            assert!(parse_inline(input, "image").is_err(), "{input}");
        }
        let wav = b"RIFF\0\0\0\0WAVE";
        assert!(
            parse_inline(
                &format!("data:audio/wav;base64,{}", STANDARD.encode(wav)),
                "audio"
            )
            .is_ok()
        );
    }

    #[test]
    fn remote_media_trust_matches_the_upstream_public_hosts() {
        for host in [
            "cdn.sogni.ai",
            "a.sogni.ai",
            "x.s3.amazonaws.com",
            "x.s3.us-west-2.amazonaws.com",
            "s3.us-east-1.amazonaws.com",
            "x.cloudfront.net",
        ] {
            assert!(trusted_url(&format!("https://{host}/media")).is_some());
        }
        for input in [
            "https://sogni.ai.evil.test/a",
            "http://cdn.sogni.ai/a",
            "https://user:pass@cdn.sogni.ai/a",
            "https://localhost/a",
            "https://127.0.0.1/a",
        ] {
            assert!(trusted_url(input).is_none(), "{input}");
        }
    }
}
