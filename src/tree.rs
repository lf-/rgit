//! Functions for handling git trees as tree structures
use anyhow::{Context, Result};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::mem;
use thiserror::Error;

use crate::index::{Index, IndexEntry};
use crate::objects::{File, Id, Object, Repo, Tree};

#[derive(Error, Debug)]
pub enum TreeError {
    #[error("Got an ID {0} that was not for the expected object type")]
    BadId(Id),
}

pub type SubTree = BTreeMap<String, TreeEntry>;

/// A recursive tree structure based on BTreeMap to represent a repository tree
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TreeEntry {
    Blob(Id),
    Tree(Id),
    SubTree(SubTree),
}

impl TreeEntry {
    pub fn subtree_mut(&mut self) -> Option<&mut SubTree> {
        if let TreeEntry::SubTree(st) = self {
            Some(st)
        } else {
            None
        }
    }
    pub fn subtree(&self) -> Option<&SubTree> {
        if let TreeEntry::SubTree(st) = self {
            Some(st)
        } else {
            None
        }
    }

    /// Generates the expected permissions for a file or directory in git.
    /// Panics if you call it on an unflattened subtree.
    pub fn perms(&self) -> (&Id, u32) {
        // TODO: support executable files and symlinks
        match self {
            TreeEntry::Blob(id) => (id, 0o100644),
            TreeEntry::Tree(id) => (id, 0o040000),
            _ => unreachable!("asked for permissions on an unflattened tree {:?}", self),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
/// A type to represent differences between two trees
pub enum Diff {
    Different,
    ExtraInRight,
    ExtraInLeft,
}

/// Finds the differences between two flat, sorted file lists
pub fn diff_file_lists<'a, 'b, T, F>(
    left: &[(&'a str, &'b T)],
    right: &[(&'a str, &'b T)],
    comparator: F,
) -> Vec<(&'a str, Diff)>
where
    T: Eq,
    F: Fn(&'b T, &'b T) -> bool,
{
    // lists MUST be sorted, check this invariant before we make mistakes
    assert!(
        left.is_sorted_by_key(|&(name, _)| name) && right.is_sorted_by_key(|&(name, _)| name),
        "Input MUST be sorted"
    );

    let mut diffs = Vec::new();

    let mut liter = left.iter();
    let mut riter = right.iter();

    let mut lnext = liter.next();
    let mut rnext = riter.next();
    // loop through both left and right structures at once
    loop {
        match (lnext, rnext) {
            // A A
            // - -
            (None, None) => break,

            // A A
            // - B
            (None, Some((r, _))) => {
                diffs.push((*r, Diff::ExtraInRight));
                rnext = riter.next();
            }

            // A A
            // B -
            (Some((l, _)), None) => {
                diffs.push((l, Diff::ExtraInLeft));
                lnext = liter.next();
            }

            // A:1 A:1
            // ?:? ?:?
            (Some((l, li)), Some((r, ri))) => {
                match l.cmp(r) {
                    // A:1 A:1
                    // B:2 B:2
                    Ordering::Equal if comparator(li, ri) => {
                        lnext = liter.next();
                        rnext = riter.next();
                    }

                    // A:1 A:1
                    // B:2 B:3
                    Ordering::Equal => {
                        diffs.push((l, Diff::Different));
                        lnext = liter.next();
                        rnext = riter.next();
                    }

                    // A:1 A:1
                    // B:? C:?
                    Ordering::Less => {
                        diffs.push((l, Diff::ExtraInLeft));
                        // catch up
                        lnext = liter.next();
                    }

                    // A:1 A:1
                    // C:? B:?
                    Ordering::Greater => {
                        diffs.push((r, Diff::ExtraInRight));
                        // catch up
                        rnext = riter.next();
                    }
                }
            }
        }
    }
    diffs
}

/// Makes a SubTree object out of the tree in the index
pub fn index_to_tree(index: &Index) -> SubTree {
    let mut root_st = SubTree::new();

    for IndexEntry {
        name: path,
        meta: entry,
    } in index
    {
        let mut inserting_into = &mut root_st;

        let mut parts = path.split("/").peekable();
        let filename;

        // Get a reference to the SubTree of the last directory in the path
        // XXX: are there symlink bugs?
        loop {
            let part = parts.next().unwrap();
            // Exclude the last element
            if parts.peek().is_none() {
                filename = part;
                break;
            }

            inserting_into = inserting_into
                .entry(part.to_string())
                .or_insert_with(|| TreeEntry::SubTree(SubTree::new()))
                .subtree_mut()
                .expect("component was not a directory?!");
        }

        inserting_into.insert(filename.to_string(), TreeEntry::Blob(entry.id));
    }
    root_st
}

/// Load a tree by ID, ensuring that it is in fact a tree
fn tree_or_err(id: &Id, repo: &Repo) -> Result<Tree> {
    let obj = repo.open(id)?;
    match obj {
        Object::Tree(tree) => Ok(tree),
        _ => Err(anyhow::Error::new(TreeError::BadId(id.clone()))),
    }
}

/// Loads a Tree from the database and turns it into a realized SubTree structure
/// for processing
pub fn load_tree_from_disk(
    tree: Tree,
    repo: &Repo,
    base_path: &str,
    filelist: &mut Vec<(String, Id)>,
) -> Result<()> {
    // TODO: probably should limit stack depth

    for item in tree.files {
        let is_dir = item.is_dir();

        let path = if base_path == "" {
            item.name
        } else {
            [base_path, &item.name].join("/")
        };

        if is_dir {
            // if it's a directory we should recurse down and grab all its files
            load_tree_from_disk(tree_or_err(&item.id, repo)?, repo, &path, filelist)?;
        } else {
            // we can stuff the file straight into the file list
            filelist.push((path, item.id));
        }
    }
    Ok(())
}

/// Saves a *flattened* tree to disk
/// Warning: it will panic if the tree is not flat!
pub fn save_subtree_to_disk(st: &SubTree, repo: &Repo) -> Result<Id> {
    let files = st.iter().map(|(name, e)| {
        let (id, mode) = e.perms();
        File {
            id: id.clone(),
            mode,
            name: name.clone(),
        }
    });
    let tree = Tree {
        files: files.collect(),
    };
    repo.store(&tree).context("error storing file in repo")
}

/// Saves an unflattened subtree to disk
pub fn save_subtree(subtree: &mut TreeEntry, repo: &Repo) -> Result<Id> {
    for (_, st) in subtree.subtree_mut().unwrap() {
        match st {
            TreeEntry::SubTree(_) => {
                let saved = TreeEntry::Tree(save_subtree(st, repo)?);
                mem::replace(st, saved);
            }
            TreeEntry::Blob(_) | TreeEntry::Tree(_) => {
                // we don't need to save these
            }
        }
    }
    // if we've escaped this loop, there are no more subtrees in our subtree. We
    // may save it now
    save_subtree_to_disk(subtree.subtree().unwrap(), repo)
}

#[cfg(test)]
mod test {
    use super::Diff;
    use crate::objects::Id;

    #[test]
    fn test_tree_comparison() {
        let comparator = |a, b| a == b;
        let mut tree1 = Vec::new();
        let mut tree2 = Vec::new();

        let id1 = Id::from("0000000000000000000000000000000000000000").unwrap();
        let id2 = Id::from("ffffffffffffffffffffffffffffffffffffffff").unwrap();

        tree1.push(("a", &id1));
        tree2.push(("a", &id2));
        let identical = tree1.clone();
        let diffs = super::diff_file_lists(&tree1, &identical, comparator);

        // identical trees should not have any diff output
        assert_eq!(diffs.len(), 0);

        let diffs = super::diff_file_lists(&tree1, &tree2, comparator);

        // 'a' should be different
        assert_eq!(diffs, vec![("a", Diff::Different)]);

        // an extra item in left
        tree1.push(("b", &id1));
        let diffs = super::diff_file_lists(&tree1, &tree2, comparator);
        assert_eq!(
            diffs,
            vec![("a", Diff::Different), ("b", Diff::ExtraInLeft)]
        );

        // test fast-forward advance
        tree1.push(("aa", &id1));
        tree2.push(("b", &id2));

        // we only accept sorted trees
        tree1.sort_by_key(|&(name, _)| name);
        println!("tree 1: {:?}\ntree 2: {:?}", &tree1, &tree2);
        let diffs = super::diff_file_lists(&tree1, &tree2, comparator);
        assert_eq!(
            diffs,
            vec![
                ("a", Diff::Different),
                ("aa", Diff::ExtraInLeft),
                ("b", Diff::Different),
            ]
        );
    }
}
