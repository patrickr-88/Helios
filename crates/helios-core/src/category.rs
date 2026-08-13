//! File classification into the eight buckets the Category view presents.
//!
//! Classification is extension-driven and allocation-free on the hot path: the
//! scanner sees millions of names, so we lowercase into a small stack buffer
//! and match against a sorted static table rather than building a `HashMap`
//! entry or a `String` per file.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Documents = 0,
    Images = 1,
    Videos = 2,
    Audio = 3,
    Archives = 4,
    Applications = 5,
    Developer = 6,
    System = 7,
    Other = 8,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Documents,
        Category::Images,
        Category::Videos,
        Category::Audio,
        Category::Archives,
        Category::Applications,
        Category::Developer,
        Category::System,
        Category::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Documents => "documents",
            Category::Images => "images",
            Category::Videos => "videos",
            Category::Audio => "audio",
            Category::Archives => "archives",
            Category::Applications => "applications",
            Category::Developer => "developer",
            Category::System => "system",
            Category::Other => "other",
        }
    }

    pub fn from_str_name(s: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// Extension → category. Kept sorted so the lookup can binary-search; the
/// `sorted_table` test enforces that invariant.
static EXTENSIONS: &[(&str, Category)] = &[
    ("7z", Category::Archives),
    ("aac", Category::Audio),
    ("ai", Category::Images),
    ("aiff", Category::Audio),
    ("apk", Category::Applications),
    ("app", Category::Applications),
    ("avi", Category::Videos),
    ("avif", Category::Images),
    ("bmp", Category::Images),
    ("bz2", Category::Archives),
    ("c", Category::Developer),
    ("cache", Category::System),
    ("cpp", Category::Developer),
    ("cr2", Category::Images),
    ("csv", Category::Documents),
    ("dat", Category::System),
    ("db", Category::System),
    ("deb", Category::Applications),
    ("dll", Category::Applications),
    ("dmg", Category::Applications),
    ("dng", Category::Images),
    ("doc", Category::Documents),
    ("docx", Category::Documents),
    ("dylib", Category::Applications),
    ("epub", Category::Documents),
    ("exe", Category::Applications),
    ("flac", Category::Audio),
    ("flv", Category::Videos),
    ("framework", Category::Applications),
    ("gif", Category::Images),
    ("go", Category::Developer),
    ("gz", Category::Archives),
    ("h", Category::Developer),
    ("heic", Category::Images),
    ("hpp", Category::Developer),
    ("ico", Category::Images),
    ("img", Category::Archives),
    ("iso", Category::Archives),
    ("java", Category::Developer),
    ("jpeg", Category::Images),
    ("jpg", Category::Images),
    ("js", Category::Developer),
    ("json", Category::Developer),
    ("jsx", Category::Developer),
    ("kext", Category::System),
    ("key", Category::Documents),
    ("log", Category::System),
    ("m4a", Category::Audio),
    ("m4v", Category::Videos),
    ("md", Category::Documents),
    ("mkv", Category::Videos),
    ("mov", Category::Videos),
    ("mp3", Category::Audio),
    ("mp4", Category::Videos),
    ("mpg", Category::Videos),
    ("msi", Category::Applications),
    ("numbers", Category::Documents),
    ("o", Category::Developer),
    ("odt", Category::Documents),
    ("ogg", Category::Audio),
    ("pages", Category::Documents),
    ("pdf", Category::Documents),
    ("pkg", Category::Applications),
    ("plist", Category::System),
    ("png", Category::Images),
    ("ppt", Category::Documents),
    ("pptx", Category::Documents),
    ("psd", Category::Images),
    ("py", Category::Developer),
    ("rar", Category::Archives),
    ("raw", Category::Images),
    ("rb", Category::Developer),
    ("rlib", Category::Developer),
    ("rs", Category::Developer),
    ("rtf", Category::Documents),
    ("sketch", Category::Images),
    ("so", Category::Applications),
    ("sparsebundle", Category::Archives),
    ("sqlite", Category::System),
    ("svg", Category::Images),
    ("swift", Category::Developer),
    ("sys", Category::System),
    ("tar", Category::Archives),
    ("tiff", Category::Images),
    ("ts", Category::Developer),
    ("tsx", Category::Developer),
    ("txt", Category::Documents),
    ("wav", Category::Audio),
    ("webm", Category::Videos),
    ("webp", Category::Images),
    ("wma", Category::Audio),
    ("wmv", Category::Videos),
    ("xls", Category::Documents),
    ("xlsx", Category::Documents),
    ("xz", Category::Archives),
    ("yaml", Category::Developer),
    ("yml", Category::Developer),
    ("zip", Category::Archives),
    ("zst", Category::Archives),
];

/// Longest extension we bother matching; anything longer is `Other`.
const MAX_EXT: usize = 16;

/// Returns the extension of `name`, lowercased, without allocating.
fn extension_lower(name: &str, buf: &mut [u8; MAX_EXT]) -> Option<usize> {
    // Skip the leading dot of dotfiles so ".gitignore" is not read as the
    // extension "gitignore".
    let stem = name.strip_prefix('.').unwrap_or(name);
    let dot = stem.rfind('.')?;
    let ext = &stem[dot + 1..];
    if ext.is_empty() || ext.len() > MAX_EXT || !ext.is_ascii() {
        return None;
    }
    let bytes = ext.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        buf[i] = b.to_ascii_lowercase();
    }
    Some(bytes.len())
}

/// Classifies a file by name. `system_path` short-circuits everything under an
/// OS-owned prefix so that, say, a `.png` inside `/System` still reads as a
/// system file in the category breakdown.
pub fn classify(name: &str, system_path: bool) -> Category {
    if system_path {
        return Category::System;
    }
    let mut buf = [0u8; MAX_EXT];
    let Some(len) = extension_lower(name, &mut buf) else {
        return Category::Other;
    };
    let ext = std::str::from_utf8(&buf[..len]).unwrap_or("");
    match EXTENSIONS.binary_search_by(|(k, _)| (*k).cmp(ext)) {
        Ok(i) => EXTENSIONS[i].1,
        Err(_) => Category::Other,
    }
}

/// Returns the lowercased extension of a name as an owned `String`, for the
/// query layer's extension filter (cold path — allocation is fine here).
pub fn extension_of(name: &str) -> Option<String> {
    let mut buf = [0u8; MAX_EXT];
    let len = extension_lower(name, &mut buf)?;
    std::str::from_utf8(&buf[..len]).ok().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_table() {
        assert!(
            EXTENSIONS.windows(2).all(|w| w[0].0 < w[1].0),
            "EXTENSIONS must stay sorted and duplicate-free for binary_search"
        );
    }

    #[test]
    fn classifies_by_extension_case_insensitively() {
        assert_eq!(classify("Holiday.JPG", false), Category::Images);
        assert_eq!(classify("archive.tar.gz", false), Category::Archives);
        assert_eq!(classify("notes", false), Category::Other);
        assert_eq!(classify("weird.unknownext", false), Category::Other);
    }

    #[test]
    fn dotfiles_are_not_extensions() {
        assert_eq!(classify(".gitignore", false), Category::Other);
        assert_eq!(extension_of(".gitignore"), None);
        assert_eq!(extension_of(".hidden.png").as_deref(), Some("png"));
    }

    #[test]
    fn system_paths_override_extension() {
        assert_eq!(classify("icon.png", true), Category::System);
    }
}
