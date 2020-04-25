use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset, Local};
use std::ascii;
use std::env;
use std::fs::OpenOptions;
use std::io;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use walkdir::WalkDir;

use crate::args;
use crate::args::OutputType;
use crate::index;
use crate::objects::{Blob, Commit, Id, NameEntry, Object, Repo};
use crate::tree::{load_tree_from_disk, save_subtree, SubTree, TreeEntry};
use crate::util::GitPath;

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

pub(crate) fn add(files: Vec<String>) -> Result<()> {
    let repo = Repo::new().context("failed to find repo")?;
    let mut my_index = repo.index()?;

    for file in files {
        let file = Path::new(&file);
        if !file.exists() {
            return Err(anyhow!("Path {} does not exist!", file.display()));
        }

        let wd = WalkDir::new(file).follow_links(false);

        'inner: for f in wd {
            let f: walkdir::DirEntry = f?;
            if f.file_type().is_dir() {
                continue 'inner;
            }

            let path = repo.repo_relative(f.path())?;

            let path = path.to_git_path();

            index::add_to_index(&mut my_index, &path, &repo)?;
        }
    }
    let unsorted = my_index.clone();
    my_index.sort_by(|(name, _), (name2, _)| name.cmp(name2));
    assert_eq!(unsorted, my_index);

    repo.write_index(&my_index)?;

    Ok(())
}

pub(crate) fn status() -> Result<()> {
    let repo = Repo::new().context("failed to find repo")?;

    let head = repo
        .head()?
        .context("Repo does not have a HEAD. You can commit to create one.")?;

    let cmt = match repo.open(&head)? {
        Object::Commit(cmt) => cmt,
        _ => return Err(anyhow!("HEAD was not a commit")),
    };

    let tree = match repo.open(&cmt.tree)? {
        Object::Tree(t) => t,
        _ => return Err(anyhow!("commit tree was not a tree")),
    };

    // Optimization: use the cached subtree extension

    let realized = load_tree_from_disk(tree, &repo)?;
    Ok(())
}

// -----------------------------------------
// Plumbing Commands
// -----------------------------------------

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

/// Create a new tree object, ready to commit.
pub(crate) fn new_tree(paths: Vec<String>) -> Result<()> {
    let repo = Repo::new().context("failed to find .git")?;
    let paths = paths.iter().map(|p| Path::new(p)).collect::<Vec<&Path>>();
    for &path in &paths {
        // TODO: support handling directories. probably requires thought re:
        // symlinks
        if !path.is_file() {
            return Err(anyhow!("{} is not a file", &path.display()));
        }
    }

    // this is canonicalized because Windows puts a \\?\ before the path when
    // `.canonicalize()` is called which gets caught in the machinery of
    // strip_prefix, so we ensure the thing we're stripping also has the same
    // artifact
    let mut tree = TreeEntry::SubTree(SubTree::new());

    for &path in &paths {
        let repo_relative = repo.repo_relative(path)?;

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

    let id = save_subtree(&mut tree, &repo)?;
    println!("tree {}", id);

    Ok(())
}

pub(crate) fn catfile(id: &str, output: OutputType) -> Result<()> {
    let id = Id::from(id).context("invalid ID format")?;
    let repo = Repo::new().context("failed to find repo")?;
    let mut h = repo.open_object_raw(&id)?;
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
