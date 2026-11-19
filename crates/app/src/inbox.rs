//! Folders handed to diskmap from outside (Finder service, `open -a diskmap <dir>`, file managers).
//! Platform callbacks run without `&mut App`, so they post here and a task in `main` drains it.
use std::path::PathBuf;
use std::sync::OnceLock;

type Channel = (async_channel::Sender<PathBuf>, async_channel::Receiver<PathBuf>);

fn channel() -> &'static Channel {
    static CHANNEL: OnceLock<Channel> = OnceLock::new();
    CHANNEL.get_or_init(async_channel::unbounded)
}

pub fn send(path: PathBuf) {
    channel().0.try_send(path).ok();
}

pub fn receiver() -> async_channel::Receiver<PathBuf> {
    channel().1.clone()
}

/// GPUI's open-URLs callback: only local `file://` URLs are scanned.
pub fn send_urls(urls: Vec<String>) {
    if let Some(p) = urls.iter().find_map(|u| file_url_to_path(u)) {
        send(p);
    }
}

pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = percent_decode(rest.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%'
            && let (Some(h), Some(l)) = (s.get(i + 1).and_then(|&b| hex(b)), s.get(i + 2).and_then(|&b| hex(b)))
        {
            out.push((h << 4) | l);
            i += 3;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Explorer passes `"%V"`; for a drive root that is `"C:\"`, whose `\"` Windows reads as an
/// escaped quote, so the argument arrives as `C:"`.
#[cfg(windows)]
pub fn normalize_arg(arg: String) -> String {
    let arg = arg.trim_end_matches('"').to_string();
    if arg.ends_with(':') { arg + "\\" } else { arg }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_urls() {
        assert_eq!(file_url_to_path("file:///Users/juan/Downloads/"), Some(PathBuf::from("/Users/juan/Downloads/")));
        assert_eq!(file_url_to_path("file:///tmp/a%20b/%C3%B1"), Some(PathBuf::from("/tmp/a b/ñ")));
        assert_eq!(file_url_to_path("file://localhost/Volumes/X"), Some(PathBuf::from("/Volumes/X")));
        assert_eq!(file_url_to_path("file:///bad%zz"), Some(PathBuf::from("/bad%zz")));
        assert_eq!(file_url_to_path("https://example.com/"), None);
        assert_eq!(file_url_to_path("file://server/share"), None);
    }
}
