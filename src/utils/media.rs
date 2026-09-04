use std::path::Path;

#[must_use]
pub fn detect_content_type(path: Option<&Path>, bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Some("image/jpeg".into());
    }
    if bytes.starts_with(b"\x89PNG") {
        return Some("image/png".into());
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif".into());
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp".into());
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some("audio/wav".into());
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        return Some("audio/mpeg".into());
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = String::from_utf8_lossy(&bytes[8..12]).to_ascii_lowercase();
        if brand.contains("m4a") || brand.contains("m4b") {
            return Some("audio/mp4".into());
        }
        if brand.contains("qt") {
            return Some("video/quicktime".into());
        }
        return Some("video/mp4".into());
    }
    path.and_then(|path| {
        mime_guess::from_path(path)
            .first_raw()
            .map(ToOwned::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_media_signatures_before_extensions() {
        let cases: &[(&[u8], &str)] = &[
            (b"\xff\xd8\xffmore", "image/jpeg"),
            (b"\x89PNG\r\n\x1a\n", "image/png"),
            (b"GIF89a", "image/gif"),
            (b"RIFF\0\0\0\0WEBP", "image/webp"),
            (b"RIFF\0\0\0\0WAVE", "audio/wav"),
            (b"ID3music", "audio/mpeg"),
            (b"\0\0\0\x18ftypM4A ", "audio/mp4"),
            (b"\0\0\0\x18ftypqt  ", "video/quicktime"),
            (b"\0\0\0\x18ftypisom", "video/mp4"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(detect_content_type(None, bytes).as_deref(), Some(*expected));
        }
    }
}
