//! What a fetched or on-disk file is, by content type and then by its first bytes: the extension a
//! saved body should carry (so `read` can open it) and whether it is an image, audio or video.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MediaKind {
    Image,
    Audio,
    Video,
}

impl MediaKind {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Image => "an image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

pub(crate) fn kind_of_ext(ext: &str) -> Option<MediaKind> {
    match ext.to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "avif" | "tif" | "tiff" => Some(MediaKind::Image),
        "mp3" | "wav" | "m4a" | "aac" | "ogg" | "oga" | "opus" | "flac" | "weba" | "aiff" => Some(MediaKind::Audio),
        "mp4" | "webm" | "mov" | "mkv" | "avi" | "mpeg" | "mpg" | "m4v" => Some(MediaKind::Video),
        _ => None,
    }
}

/// Content-type token (lowercased, parameters allowed) to the extension of the format it names.
fn ext_from_content_type(ct: &str) -> Option<&'static str> {
    const TABLE: &[(&str, &str)] = &[
        ("image/png", "png"),
        ("image/jpeg", "jpg"),
        ("image/jpg", "jpg"),
        ("image/pjpeg", "jpg"),
        ("image/gif", "gif"),
        ("image/webp", "webp"),
        ("image/bmp", "bmp"),
        ("image/svg", "svg"),
        ("image/avif", "avif"),
        ("image/tiff", "tiff"),
        ("image/x-icon", "ico"),
        ("image/vnd.microsoft.icon", "ico"),
        ("audio/mpeg", "mp3"),
        ("audio/mp3", "mp3"),
        ("audio/wav", "wav"),
        ("audio/x-wav", "wav"),
        ("audio/wave", "wav"),
        ("audio/mp4", "m4a"),
        ("audio/x-m4a", "m4a"),
        ("audio/aac", "aac"),
        ("audio/ogg", "ogg"),
        ("audio/opus", "opus"),
        ("audio/flac", "flac"),
        ("audio/x-flac", "flac"),
        ("audio/webm", "weba"),
        ("video/mp4", "mp4"),
        ("video/webm", "webm"),
        ("video/quicktime", "mov"),
        ("video/x-matroska", "mkv"),
        ("video/x-msvideo", "avi"),
        ("video/mpeg", "mpg"),
        ("application/pdf", "pdf"),
        ("application/zip", "zip"),
        ("application/gzip", "gz"),
        ("application/x-gzip", "gz"),
        ("application/x-zip", "zip"),
        ("application/vnd.openxmlformats-officedocument.spreadsheetml", "xlsx"),
        ("application/vnd.ms-excel", "xls"),
        ("application/vnd.openxmlformats-officedocument.wordprocessingml", "docx"),
        ("application/msword", "doc"),
        ("application/vnd.openxmlformats-officedocument.presentationml", "pptx"),
        ("application/vnd.ms-powerpoint", "ppt"),
    ];
    TABLE.iter().find(|(k, _)| ct.contains(k)).map(|(_, ext)| *ext)
}

fn ext_from_magic(b: &[u8]) -> Option<&'static str> {
    let at = |i: usize, s: &[u8]| b.get(i..i + s.len()) == Some(s);
    if at(0, b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if at(0, b"\xFF\xD8\xFF") {
        Some("jpg")
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        Some("gif")
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        Some("webp")
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        Some("wav")
    } else if at(0, b"RIFF") && at(8, b"AVI ") {
        Some("avi")
    } else if at(0, b"OggS") {
        Some("ogg")
    } else if at(0, b"fLaC") {
        Some("flac")
    } else if at(0, b"ID3") {
        Some("mp3")
    } else if at(4, b"ftyp") {
        Some(if at(8, b"M4A ") { "m4a" } else if at(8, b"qt  ") { "mov" } else { "mp4" })
    } else if at(0, b"\x1A\x45\xDF\xA3") {
        Some("webm")
    } else if at(0, b"%PDF") {
        Some("pdf")
    } else if at(0, b"PK\x03\x04") {
        Some("zip")
    } else {
        None
    }
}

/// Extension for a non-text response body, or `None` when it is text to decode. Content type
/// first, then the leading bytes (not for a type that says text, where a signature is a fluke).
pub(crate) fn binary_ext(content_type: &str, bytes: &[u8]) -> Option<&'static str> {
    let ct = content_type.to_ascii_lowercase();
    if let Some(ext) = ext_from_content_type(&ct) {
        return Some(ext);
    }
    let textual = ["text/", "json", "xml", "html", "javascript"].iter().any(|k| ct.contains(k));
    if !textual && let Some(ext) = ext_from_magic(bytes) {
        return Some(ext);
    }
    let media_prefix = ["image/", "audio/", "video/"].iter().any(|p| ct.starts_with(p));
    (media_prefix || (ct.contains("octet-stream") && bytes.iter().take(1024).any(|&b| b == 0))).then_some("bin")
}

/// The media kind of a file on disk: its extension, else its first bytes.
pub(crate) fn kind_of_file(path: &std::path::Path) -> Option<MediaKind> {
    if let Some(kind) = path.extension().and_then(|e| e.to_str()).and_then(kind_of_ext) {
        return Some(kind);
    }
    use std::io::Read;
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    ext_from_magic(&head[..n]).and_then(kind_of_ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_wins_then_magic_bytes() {
        assert_eq!(binary_ext("image/png", b""), Some("png"));
        assert_eq!(binary_ext("image/jpeg; charset=binary", b""), Some("jpg"));
        assert_eq!(binary_ext("audio/mpeg", b""), Some("mp3"));
        assert_eq!(binary_ext("video/quicktime", b""), Some("mov"));
        assert_eq!(binary_ext("application/octet-stream", b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(binary_ext("", b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(binary_ext("application/octet-stream", b"\0\0\0\x18ftypM4A \0"), Some("m4a"));
        assert_eq!(binary_ext("application/octet-stream", b"OggS\0\x02"), Some("ogg"));
        assert_eq!(binary_ext("application/octet-stream", b"PK\x03\x04"), Some("zip"));
        assert_eq!(binary_ext("image/x-unheard-of", b"abc"), Some("bin"));
    }

    #[test]
    fn text_bodies_are_not_binary() {
        assert_eq!(binary_ext("text/html", b"<html>"), None);
        assert_eq!(binary_ext("text/plain", b"ID3 is a tag"), None);
        assert_eq!(binary_ext("application/json", b"{}"), None);
        assert_eq!(binary_ext("", b"plain words"), None);
    }

    #[test]
    fn files_are_classified_by_extension_then_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let named = dir.path().join("a.wav");
        std::fs::write(&named, b"x").unwrap();
        assert_eq!(kind_of_file(&named), Some(MediaKind::Audio));
        let bare = dir.path().join("clip");
        std::fs::write(&bare, b"\0\0\0\x18ftypmp42").unwrap();
        assert_eq!(kind_of_file(&bare), Some(MediaKind::Video));
        let text = dir.path().join("notes");
        std::fs::write(&text, b"hello").unwrap();
        assert_eq!(kind_of_file(&text), None);
    }
}
