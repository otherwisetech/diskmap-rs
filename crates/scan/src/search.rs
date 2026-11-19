//! Name search over a scanned subtree: case- and accent-insensitive substring match.
use crate::tree::{NodeId, Tree};

impl Tree {
    /// Descendants of `under` (not `under` itself) whose name contains `query`, ignoring case and
    /// accents ("cafe" finds "Café", also when stored decomposed). Largest first, at most `limit`.
    pub fn search(&self, under: NodeId, query: &str, limit: usize) -> Vec<NodeId> {
        let mut needle = String::new();
        fold_into(query.trim(), &mut needle);
        if needle.is_empty() || limit == 0 {
            return Vec::new();
        }
        let ascii_needle = needle.is_ascii();
        let mut buf = String::new();
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.node(under).children.clone();
        while let Some(id) = stack.pop() {
            let n = self.node(id);
            if n.removed {
                continue;
            }
            stack.extend_from_slice(&n.children);
            let hit = if n.name.is_ascii() {
                // ascii names fold to themselves lowercased, so no buffer is needed
                ascii_needle && contains_ignore_ascii_case(n.name.as_bytes(), needle.as_bytes())
            } else {
                buf.clear();
                fold_into(&n.name, &mut buf);
                buf.contains(needle.as_str())
            };
            if hit {
                out.push(id);
            }
        }
        let by_size = |a: &NodeId, b: &NodeId| self.node(*b).size.cmp(&self.node(*a).size).then(a.cmp(b));
        if out.len() > limit {
            out.select_nth_unstable_by(limit - 1, by_size);
            out.truncate(limit);
        }
        out.sort_unstable_by(by_size);
        out
    }
}

/// `needle` must already be lowercase ASCII.
fn contains_ignore_ascii_case(hay: &[u8], needle: &[u8]) -> bool {
    needle.len() <= hay.len() && hay.windows(needle.len()).any(|w| w.iter().zip(needle).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

/// Lowercase and strip accents (Latin scripts) into `out`.
pub fn fold_into(s: &str, out: &mut String) {
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
            continue;
        }
        for l in c.to_lowercase() {
            match l {
                '\u{300}'..='\u{36f}' => {} // combining marks (macOS names are often decomposed)
                'ß' => out.push_str("ss"),
                'æ' => out.push_str("ae"),
                'œ' => out.push_str("oe"),
                _ => out.push(base_letter(l)),
            }
        }
    }
}

fn base_letter(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use crate::tree::{Node, Tree};
    use crate::Kind;
    use std::time::Instant;

    fn node(name: String, parent: Option<u32>, size: u64, kind: Kind) -> Node {
        Node { name: name.into(), parent, children: Vec::new(), size, files: 1, kind, unreadable: false, removed: false }
    }

    /// `cargo test -p diskmap-scan --release -- --ignored --nocapture search_4m`
    #[test]
    #[ignore = "timing; run in release"]
    fn search_4m_nodes() {
        const DIRS: u32 = 40_000;
        const FILES_PER: u32 = 99;
        let mut nodes = vec![node("root".into(), None, 0, Kind::Dir)];
        for d in 0..DIRS {
            let did = nodes.len() as u32;
            nodes[0].children.push(did);
            nodes.push(node(format!("project-{d}"), Some(0), 0, Kind::Dir));
            for f in 0..FILES_PER {
                let fid = nodes.len() as u32;
                nodes[did as usize].children.push(fid);
                let name = if f % 50 == 0 { format!("Café Résumé {f}.txt") } else { format!("IMG_{d:05}_{f:03}.jpg") };
                nodes.push(node(name, Some(did), u64::from(f) * 1000 + u64::from(d), Kind::Other));
            }
        }
        let t = Tree { nodes, root_path: "/r".into() };
        eprintln!("nodes: {}", t.len());
        for q in ["cafe", "img_0", "zzz-nothing", "e"] {
            let at = Instant::now();
            let r = t.search(Tree::ROOT, q, 500);
            eprintln!("{q:?}: {} hits in {:?}", r.len(), at.elapsed());
        }
    }
}
