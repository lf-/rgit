use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset, Local};
use std::ascii;
use std::collections::BTreeMap;
use std::env;
use std::fs::OpenOptions;
use std::io;
use std::io::{BufReader, Read, Write};
use std::mem;
use std::path::Path;

use crate::args;
use crate::args::OutputType;
use crate::index;
use crate::objects;
use crate::objects::{Blob, Commit, File, Id, NameEntry, Object, Repo, Tree};

pub(crate) fn init() -> Result<()> {
    if Repo::new().is_some() {
        // technically this stops us from making a repo inside a repo but that
        // is also probably a bad idea to do
        return Err(anyhow!(
            "repo already exists in this directory or its parent"
        ));
    }

    Repo::init(&env::current_dir()?)?;
    Ok(())
}

pub(crate) fn commit_tree(id: String, who: String, message: String) -> Result<()> {
    let repo = Repo::new().context("couldn't find repo")?;
    let id = Id::from(&id).context("invalid ID format")?;
    if !repo.has_id(&id) {
        return Err(anyhow!("given ID does not exist in the database"));
    }

    let obj = repo.open(&id)?;
    match obj {
        Object::Tree(_) => (),
        _ => return Err(anyhow!("given ID is not a tree"))?,
    }

    let time = Local::now();
    let offs = time.offset();
    let time = DateTime::<FixedOffset>::from_utc(time.naive_utc(), offs.clone());
    let who = NameEntry::with_time(&who, time).context("invalid `who`")?;

    let mut parents = Vec::new();
    if let Some(head) = repo.head()? {
        parents.push(head);
    }

    let commit_object = Commit {
        author: who.clone(),
        committer: who.clone(),
        message,
        tree: id,
        parents,
    };

    let commit_id = repo.store(&commit_object)?;
    repo.set_head(&commit_id)?;
    println!("HEAD is now {}", &commit_id);

    Ok(())
}

type SubTree = BTreeMap<String, TreeEntry>;

#[derive(Debug)]
enum TreeEntry {
    Blob(Id),
    Tree(Id),
    SubTree(SubTree),
}

impl TreeEntry {
    fn subtree_mut(&mut self) -> Option<&mut SubTree> {
        if let TreeEntry::SubTree(st) = self {
            Some(st)
        } else {
            None
        }
    }
    fn subtree(&self) -> Option<&SubTree> {
        if let TreeEntry::SubTree(st) = self {
            Some(st)
        } else {
            None
        }
    }

    /// generates the expected permissions for a file or directory in git
    /// sorta spiny, don't call it on a non-flat object
    fn perms(&self) -> (&Id, u32) {
        match self {
            TreeEntry::Blob(id) => (id, 0o100644),
            TreeEntry::Tree(id) => (id, 0o040000),
            _ => unreachable!("asked for permissions on an unflattened tree"),
        }
    }

    /// saves a *flattened* tree to disk
    /// it will panic if the tree is not flat.
    fn save(&self, repo: &Repo) -> Result<Id> {
        let st = self
            .subtree()
            .expect("can only save things that are not already on disk");

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
        repo.store(&tree)
    }
}

pub(crate) fn new_tree(paths: Vec<String>) -> Result<()> {
    let repo = Repo::new().context("failed to find .git")?;
    let paths = paths.iter().map(|p| Path::new(p)).collect::<Vec<&Path>>();
    for &path in &paths {
        // TODO: support handling directories. probably requires thought re:
        // symlinks
        if !path.is_file() {
            return Err(anyhow!(
                "one or more of the given paths does not exist or is not a file"
            ));
        }
    }

    // this is canonicalized because Windows puts a \\?\ before the path when
    // `.canonicalize()` is called which gets caught in the machinery of
    // strip_prefix, so we ensure the thing we're stripping also has the same
    // artifact
    let tree_root = repo.tree_root().canonicalize()?;
    let mut tree = TreeEntry::SubTree(SubTree::new());

    for &path in &paths {
        let canonical = path.canonicalize()?;
        let repo_relative = canonical.strip_prefix(&tree_root)?;

        let blob = Blob::new_from_disk(path)
            .context(anyhow!("failed to read blob {} from disk", &path.display()))?;
        let blob = repo.store(&blob)?;

        let mut next_tree = &mut tree;

        for part in repo_relative.parent().unwrap() {
            let part = part
                .to_str()
                .context("XXX: only unicode paths are supported")?;

            next_tree = next_tree
                .subtree_mut()
                .unwrap()
                .entry(part.to_owned())
                .or_insert_with(|| TreeEntry::SubTree(SubTree::new()));
        }

        let filename = path
            .file_name()
            .unwrap()
            .to_str()
            .context("XXX: only unicode filenames are supported")?;

        next_tree
            .subtree_mut()
            .unwrap()
            .insert(filename.to_owned(), TreeEntry::Blob(blob));
    }

    // bake the tree. Depth first search time!
    let mut remaining = Vec::new();
    remaining.push(&mut tree);
    while remaining.len() > 0 {
        let entry = remaining.pop().unwrap();

        let entry = entry;
        if let TreeEntry::SubTree(st) = entry {
            let seen_st = st.iter().any(|(_, elem)| elem.subtree().is_some());

            if seen_st {
                // this subtree has more subtrees in it, deal with it later
                for (_, elem) in entry.subtree_mut().unwrap() {
                    if elem.subtree().is_some() {
                        remaining.push(elem);
                    }
                }
            } else {
                // we are at full depth, emit a Tree for this subtree
                let tree = entry.save(&repo)?;

                mem::replace(entry, TreeEntry::Tree(tree));
            }
        }
    }
    println!("Made tree {}", tree.save(&repo)?);

    Ok(())
}

pub(crate) fn catfile(id: &str, output: OutputType) -> Result<()> {
    let id = Id::from(id).context("invalid ID format")?;
    let repo = Repo::new().context("failed to find repo")?;
    let mut h = repo.open_object(&id)?;
    match output {
        OutputType::Raw => {
            io::copy(&mut h, &mut io::stdout())?;
        }
        OutputType::Quoted => {
            let mut buf = Vec::new();
            h.read_to_end(&mut buf)?;
            let mut s = Vec::new();
            for c in buf {
                s.extend(ascii::escape_default(c));
            }
            io::stdout().write_all(&s)?;
        }
        OutputType::Debug => {
            print!("{:#?}", repo.open(&id)?);
        }
    }
    Ok(())
}

pub(crate) fn debug(what: args::DebugType) -> Result<()> {
    let repo = Repo::new().context("failed to find repo")?;

    match what {
        args::DebugType::Index => {
            let indexfile = repo.root.join("index");

            let h = OpenOptions::new()
                .read(true)
                .open(indexfile)
                .context("failed opening index file")?;
            println!("{:#x?}", index::parse(BufReader::new(h))?);
        }
    }
    Ok(())
}
