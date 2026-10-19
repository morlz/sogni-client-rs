use std::path::Path;

use sogni_client::detect_content_type;

#[test]
fn reference_container_signatures_preserve_the_public_detector_contract() {
    // Public no-slot defaults and existing formats match the upstream reference
    // content-type vectors; asset roles supply ambiguous audio/video typing.
    let cases: &[(&[u8], Option<&str>)] = &[
        (b"fLaCfixture-content", Some("audio/flac")),
        (b"OggSfixture-content", Some("audio/ogg")),
        (b"\x1a\x45\xdf\xa3fixture-content", Some("video/webm")),
        (b"RIFF\0\0\0\0WAVEfixture", Some("audio/wav")),
        (b"ID3fixture-content", Some("audio/mpeg")),
        (b"\xff\xfbfixture-content", Some("audio/mpeg")),
        (b"\0\0\0\x20ftypM4A fixture", Some("audio/mp4")),
        (b"\0\0\0\x20ftypisomfixture", Some("video/mp4")),
        (b"\x89PNGfixture-content", Some("image/png")),
        (b"\xff\xd8\xfffixture-content", Some("image/jpeg")),
        (b"GIF89afixture-content", Some("image/gif")),
        (b"RIFF\0\0\0\0WEBPfixture", Some("image/webp")),
        (b"unknown-reference", None),
        (b"fLa", None),
        (b"Ogg", None),
        (b"\x1a\x45\xdf", None),
    ];
    for (bytes, expected) in cases {
        assert_eq!(detect_content_type(None, bytes).as_deref(), *expected);
        if expected.is_some() {
            assert_eq!(
                detect_content_type(Some(Path::new("mislabeled.mp3")), bytes).as_deref(),
                *expected,
                "known bytes take precedence over a file extension"
            );
        }
    }
    assert_eq!(
        detect_content_type(Some(Path::new("unknown.wav")), b"unknown-reference").as_deref(),
        Some("audio/wav"),
        "unknown bytes retain the existing extension fallback"
    );
}
