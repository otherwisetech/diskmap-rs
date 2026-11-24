//! Arena of scanned entries. Ids stay valid after [`Tree::remove`] (the node is only detached).
use crate::Kind;
use std::path::{Path, PathBuf};

pub type NodeId = u32;

#[derive(Debug, Clone)]
pub struct Node {
    pub name: Box<str>,
    pub parent: Option<NodeId>,
    /// Sorted by size, largest first.
    pub children: Vec<NodeId>,
    /// Bytes allocated on disk (hard links counted once), including descendants.
    pub size: u64,
    /// Regular files and symlinks, including descendants.
    pub files: u64,
    pub kind: Kind,
    /// Directory (or package) we could not fully read.
    pub unreadable: bool,
    pub removed: bool,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, Kind::Dir | Kind::Package)
    }
}

#[derive(Debug, Clone)]
pub struct Tree {
    pub(crate) nodes: Vec<Node>,
    pub(crate) root_path: PathBuf,
}

impl Tree {
    pub const ROOT: NodeId = 0;

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Absolute path of a node.
    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut parts = Vec::new();
        let mut cur = id;
        while let Some(p) = self.node(cur).parent {
            parts.push(&*self.node(cur).name);
            cur = p;
        }
        let mut path = self.root_path.clone();
        path.extend(parts.into_iter().rev());
        path
    }

    /// The node at `path` (absolute, inside the root), if the scan has it and it wasn't removed.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let rel = path.strip_prefix(&self.root_path).ok()?;
        let mut cur = Tree::ROOT;
        for part in rel.components() {
            let name = part.as_os_str().to_str()?;
            cur = *self.node(cur).children.iter().find(|&&c| &*self.node(c).name == name)?;
        }
        (!self.node(cur).removed).then_some(cur)
    }

    /// Root first, `id` last.
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut v = vec![id];
        let mut cur = id;
        while let Some(p) = self.node(cur).parent {
            v.push(p);
            cur = p;
        }
        v.reverse();
        v
    }

    pub fn is_ancestor(&self, ancestor: NodeId, mut id: NodeId) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.node(id).parent {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    /// Detach a node (e.g. after it was moved to the trash) and update every ancestor's totals.
    /// Removing the root is a no-op.
    pub fn remove(&mut self, id: NodeId) {
        let Some(parent) = self.node(id).parent else { return };
        if self.node(id).removed {
            return;
        }
        let (size, files) = (self.node(id).size, self.node(id).files);
        self.nodes[id as usize].removed = true;
        self.nodes[parent as usize].children.retain(|&c| c != id);
        let mut cur = Some(parent);
        while let Some(p) = cur {
            let n = &mut self.nodes[p as usize];
            n.size -= size;
            n.files -= files;
            cur = n.parent;
        }
        // keep sibling order sorted after the sizes changed
        for a in self.ancestors(parent) {
            let mut kids = std::mem::take(&mut self.nodes[a as usize].children);
            kids.sort_by(|x, y| self.nodes[*y as usize].size.cmp(&self.nodes[*x as usize].size));
            self.nodes[a as usize].children = kids;
        }
    }

    /// The `n` biggest regular files under `id`, largest first.
    pub fn largest_files(&self, id: NodeId, n: usize) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = Vec::new();
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            let node = self.node(cur);
            if node.is_dir() {
                stack.extend(node.children.iter().copied());
            } else {
                out.push(cur);
            }
        }
        out.sort_by_key(|&id| std::cmp::Reverse(self.node(id).size));
        out.truncate(n);
        out
    }
}
