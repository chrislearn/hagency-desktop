//! Filename-to-MIME inference, ported 1:1 from the retained TypeScript
//! product (bridge-matrix.js:3204-3251). Same map, same fallback, same
//! image-extension regex, so agent-sent files keep the TS-visible MIME and
//! msgtype (m.image vs m.file) behaviour.

/// TS EXT_MIME_MAP (bridge-matrix.js:3204-3229). Do not reorder pairs: the
/// first matching extension wins exactly as in TS.
const EXT_MIME_MAP: &[(&str, &str)] = &[
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".png", "image/png"),
    (".webp", "image/webp"),
    (".gif", "image/gif"),
    (".svg", "image/svg+xml"),
    (".avif", "image/avif"),
    (".heic", "image/heic"),
    (".heif", "image/heif"),
    (".bmp", "image/bmp"),
    (".txt", "text/plain"),
    (".md", "text/markdown"),
    (".pdf", "application/pdf"),
    (".json", "application/json"),
    (".csv", "text/csv"),
    (".zip", "application/zip"),
    (".tar", "application/x-tar"),
    (".gz", "application/gzip"),
];

/// TS normalizeMimeType (bridge-matrix.js:3232-3238): trim, lowercase, then
/// require `type/subtype` with the TS character classes. None otherwise.
pub fn normalize_mime_type(value: Option<&str>) -> Option<String> {
    let mime = value?.trim().to_ascii_lowercase();
    if mime.is_empty() {
        return None;
    }
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"!#$&^_.+-".contains(&c))
    };
    let (major, sub) = mime.split_once('/')?;
    if valid(major) && valid(sub) {
        Some(mime)
    } else {
        None
    }
}

fn extension(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.rfind('.') {
        Some(at) => lower[at..].to_string(),
        None => String::new(),
    }
}

/// TS guessMimeTypeFromName (bridge-matrix.js:3240-3243): map lookup by
/// extension, else application/octet-stream.
pub fn guess_mime_type_from_name(name: &str) -> String {
    let ext = extension(name);
    EXT_MIME_MAP
        .iter()
        .find(|(candidate, _)| *candidate == ext)
        .map(|(_, mime)| (*mime).to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// TS inferAttachmentKind (bridge-matrix.js:3245-3251): explicit kind wins,
/// then image/* MIME, then the TS image-extension regex, else "file".
pub fn infer_attachment_kind(kind: Option<&str>, mime: Option<&str>, name: &str) -> &'static str {
    if let Some(kind) = kind {
        if kind == "image" {
            return "image";
        }
        if kind == "file" {
            return "file";
        }
    }
    if mime.is_some_and(|m| m.starts_with("image/")) {
        return "image";
    }
    let lower = name.to_ascii_lowercase();
    let ext = extension(&lower);
    if matches!(
        ext.as_str(),
        ".png"
            | ".jpg"
            | ".jpeg"
            | ".gif"
            | ".webp"
            | ".bmp"
            | ".svg"
            | ".avif"
            | ".heic"
            | ".heif"
            | ".tif"
            | ".tiff"
    ) {
        return "image";
    }
    "file"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_mime_vectors() {
        // Mirrors the TS map exactly.
        assert_eq!(guess_mime_type_from_name("photo.JPG"), "image/jpeg");
        assert_eq!(guess_mime_type_from_name("a.tar.gz"), "application/gzip");
        assert_eq!(
            guess_mime_type_from_name("noext"),
            "application/octet-stream"
        );
        assert_eq!(guess_mime_type_from_name("结果.txt"), "text/plain");
        assert_eq!(
            normalize_mime_type(Some("  IMAGE/PNG ")),
            Some("image/png".into())
        );
        assert_eq!(normalize_mime_type(Some("not-a-mime")), None);
        assert_eq!(normalize_mime_type(None), None);
    }

    #[test]
    fn ts_kind_vectors() {
        assert_eq!(infer_attachment_kind(Some("image"), None, "x.txt"), "image");
        assert_eq!(infer_attachment_kind(Some("file"), None, "x.png"), "file");
        assert_eq!(infer_attachment_kind(None, Some("image/png"), "x"), "image");
        assert_eq!(infer_attachment_kind(None, None, "图.HEIC"), "image");
        assert_eq!(infer_attachment_kind(None, None, "doc.pdf"), "file");
        assert_eq!(infer_attachment_kind(None, None, "photo.tiff"), "image");
    }
}
