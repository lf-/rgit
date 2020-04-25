use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::mem;
/// Functions for handling git trees as tree structures
use std::path::Path;
use thiserror::Error;

use crate::index::Index;
use crate::objects::{File, Id, Object, Repo, Tree};

#[derive(Error, Debug)]
pub enum TreeError {
    #[error("Got an ID {0} that was not for the expected object type")]
    BadId(Id),
}

pub type SubTree = BTreeMap<String, TreeEntry>;

/// A recursive tree structure based on BTreeMap to represent a repository tree
#[derive(Debug)]
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

/// Makes a SubTree object out of the tree in the index
pub fn index_to_tree(index: &Index) -> SubTree {
    let mut root_st = SubTree::new();

    for (path, entry) in index {
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
pub fn load_tree_from_disk(tree: Tree, repo: &Repo) -> Result<SubTree> {
    // TODO: probably should limit stack depth

    let mut st = SubTree::new();
    for item in tree.files {
        // silly borrow checker nonsense
        let is_dir = item.is_dir();
        st.insert(
            item.name,
            match is_dir {
                // if it's a directory, we can grab its entries recursively
                true => {
                    TreeEntry::SubTree(load_tree_from_disk(tree_or_err(&item.id, repo)?, repo)?)
                }

                // if it's a file we can put it directly into our tree
                false => TreeEntry::Blob(item.id),
            },
        );
    }
    Ok(st)
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
