//! Coarse file categories, used to colour the treemap.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Dir,
    /// Bundles that are directories on disk but one thing to the user (`.app`, `.photoslibrary`…).
    Package,
    Video,
    Image,
    Audio,
    Archive,
    Code,
    Document,
    Other,
}

impl Kind {
    /// Categories shown in the legend, in display order.
    pub const LEGEND: [Kind; 8] = [Kind::Video, Kind::Image, Kind::Audio, Kind::Archive, Kind::Code, Kind::Document, Kind::Package, Kind::Other];

    /// Stable id, used for i18n keys (`app.kind.<id>`) and colours.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Dir => "dir",
            Kind::Package => "package",
            Kind::Video => "video",
            Kind::Image => "image",
            Kind::Audio => "audio",
            Kind::Archive => "archive",
            Kind::Code => "code",
            Kind::Document => "document",
            Kind::Other => "other",
        }
    }

    pub fn for_file(name: &str) -> Kind {
        match ext(name).as_deref() {
            Some(e) => match e {
                "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" | "mts" | "braw" | "r3d" => Kind::Video,
                "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "psd" | "svg" | "exr" | "avif" => Kind::Image,
                "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" | "opus" | "aiff" | "aif" | "alac" | "mid" => Kind::Audio,
                "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "dmg" | "iso" | "pkg" | "deb" | "rpm" | "img" | "vmdk" | "qcow2" | "vdi" | "ipa" | "apk" | "jar" => Kind::Archive,
                "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "go" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "java" | "kt" | "swift" | "rb" | "php" | "cs" | "sh" | "json" | "toml" | "yaml" | "yml" | "html" | "css" | "scss" | "sql" | "o" | "a" | "rlib" | "rmeta" | "so" | "dylib" | "dll" | "wasm" | "class" | "pyc" | "lock" | "map" => Kind::Code,
                "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "txt" | "md" | "rtf" | "csv" | "epub" | "pages" | "numbers" | "key" | "ged" => Kind::Document,
                _ => Kind::Other,
            },
            None => Kind::Other,
        }
    }

    pub fn for_dir(name: &str) -> Kind {
        match ext(name).as_deref() {
            Some("app" | "photoslibrary" | "musiclibrary" | "fcpbundle" | "logicx" | "bundle" | "framework" | "xcarchive" | "vmwarevm" | "pvm" | "utm" | "sparsebundle") => Kind::Package,
            _ => Kind::Dir,
        }
    }
}

fn ext(name: &str) -> Option<String> {
    let (stem, e) = name.rsplit_once('.')?;
    (!stem.is_empty() && !e.is_empty()).then(|| e.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::Kind;

    #[test]
    fn classifies_by_extension() {
        assert_eq!(Kind::for_file("Holiday.MOV"), Kind::Video);
        assert_eq!(Kind::for_file("a.tar.gz"), Kind::Archive);
        assert_eq!(Kind::for_file("main.rs"), Kind::Code);
        assert_eq!(Kind::for_file(".bashrc"), Kind::Other);
        assert_eq!(Kind::for_file("README"), Kind::Other);
        assert_eq!(Kind::for_dir("Safari.app"), Kind::Package);
        assert_eq!(Kind::for_dir("src"), Kind::Dir);
    }
}
