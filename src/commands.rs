use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset, Local};
use std::ascii;
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::args;
use crate::args::OutputType;
use crate::index;
use crate::objects::{Blob, Commit, Id, NameEntry, Object, Repo};
use crate::rev;
use crate::tree::{
    diff_file_lists, diff_trees, index_to_tree, load_tree_from_disk, save_subtree, Diff, SubTree,
    TreeEntry,
};
use crate::util::GitPath;
use index::IndexEntry;

/// initialize a repo in the working directory
pub fn init() -> Result<()> {
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

/// add files to the index
pub fn add(files: Vec<String>) -> Result<()> {
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
            if path.is_none() {
                warn!(
                    "Skipping adding {:?} because it contains invalid UTF-8",
                    f.path()
                );
                continue 'inner;
            }
            let path = path.unwrap();

            index::add_to_index(&mut my_index, &path, &repo)?;
        }
    }
    let unsorted = my_index.clone();
    my_index.sort_by(|IndexEntry { name, .. }, IndexEntry { name: name2, .. }| name.cmp(name2));
    assert_eq!(unsorted, my_index);

    repo.write_index(&my_index)?;

    Ok(())
}

/// commit the changes staged in the index
pub fn commit(who: String, message: String) -> Result<()> {
    let repo = Repo::new().context("failed to find repo")?;

    let index_tree = index_to_tree(&repo.index()?);
    let id = save_subtree(&mut TreeEntry::SubTree(index_tree), &repo)?;
    commit_tree(id, who, message)
}

/// A Thing in the git repo
enum DiffTarget {
    /// Canonical path to the file
    File(String),
    /// Commit ID
    Commit(Id),
}

/// Finds what `name` is referencing
fn diff_what_is<'a>(name: &'a str, repo: &Repo) -> (&'a str, Option<DiffTarget>) {
    // first try interpreting it as a file name
    let file = Path::new(name);
    let fname = if file.exists() { Some(file) } else { None };
    if let Some(path) = fname {
        // potentially sinful unwraps
        return (
            name,
            Some(DiffTarget::File(
                repo.repo_relative(path).unwrap().to_git_path().unwrap(),
            )),
        );
    }

    // then try finding it as a ref
    if let Ok(rev) = rev::parse(name, repo) {
        return (name, Some(DiffTarget::Commit(rev)));
    }
    (name, None)
}

/// diff two references.
pub fn diff(args::Diff { things, cached }: args::Diff) -> Result<()> {
    let repo = Repo::new().context("failed to find git repo")?;

    let typed_things = things.iter().map(|thing| diff_what_is(thing, &repo));

    let mut commits = Vec::with_capacity(2);
    let mut files = Vec::new();
    for (name, thing) in typed_things {
        if thing.is_none() {
            return Err(anyhow!("Failed to resolve {} to a file or revision", name));
        }
        let thing = thing.unwrap();
        match thing {
            DiffTarget::Commit(id) => {
                // we can only meaningfully diff two commits
                commits.push(id);
                if commits.len() > 2 {
                    return Err(anyhow!("Got too many commits"));
                }
                if files.len() > 0 {
                    return Err(anyhow!("Got a file prior to commits"));
                }
            }
            DiffTarget::File(relative) => {
                files.push(relative);
            }
        }
    }

    // If we have no commits, we should compare working tree to HEAD
    if commits.len() == 0 {
        // default to looking at HEAD
        // we don't support diffing against a nonexistent HEAD because my HEAD
        // hurts too much for this right now
        commits.push(rev::parse("HEAD", &repo)?);
    }
    debug!("diffing {:?} commits for {:?} files", &commits, &files);
    diff_trees(&commits[0], &commits[1], "", &repo)?;

    // let id_a = rev::parse(&ref_a, &repo).context("Finding A reference")?;
    // let id_b = rev::parse(&ref_b, &repo).context("Finding B reference")?;

    // let tree_a = repo
    //     .open(&id_a)
    //     .context("Opening tree A")?
    //     .commit()
    //     .with_context(|| format!("Tree A ref {} did not point to a commit", &ref_a))?
    //     .tree;
    // let tree_a = repo
    //     .open(&tree_a)
    //     .context("Opening tree A")?
    //     .tree()
    //     .context("Tree A is not a tree")?;

    // let tree_b = repo
    //     .open(&id_b)
    //     .context("Opening tree B")?
    //     .commit()
    //     .with_context(|| format!("Tree B ref {} did not point to a commit", &ref_b))?
    //     .tree;
    // let tree_b = repo
    //     .open(&tree_b)
    //     .context("Opening tree B")?
    //     .tree()
    //     .context("Tree B is not a tree")?;

    Ok(())
}

/// get the changes between the working directory ~ index and the index ~ HEAD
pub fn status() -> Result<()> {
    let repo = Repo::new().context("failed to find repo")?;

    let head = repo.head()?;

    let cmt = match repo.open(&head)? {
        Object::Commit(cmt) => cmt,
        _ => return Err(anyhow!("HEAD was not a commit")),
    };

    let head_tree = match repo.open(&cmt.tree)? {
        Object::Tree(t) => t,
        _ => return Err(anyhow!("commit tree was not a tree")),
    };

    // Future optimization: use the cached subtree extension
    let mut head_filelist = Vec::new();
    load_tree_from_disk(head_tree, &repo, "", &mut head_filelist)?;

    let mut diff_head = head_filelist
        .iter()
        .map(|(ref name, ref id)| (name.as_str(), id));

    let index_filelist = repo.index()?;
    let mut diff_index = index_filelist
        .iter()
        .map(|IndexEntry { ref name, meta: ie }| (name.as_str(), &ie.id));

    let diffs = diff_file_lists(&mut diff_head, &mut diff_index);

    let sigil = |d| match d {
        // change in index
        Diff::Different(_, _) => "~",
        // missing from index (deleted vs HEAD)
        Diff::ExtraInLeft(_) => "-",
        // missing from HEAD (new in index)
        Diff::ExtraInRight(_) => "+",
    };

    println!("Changes to commit:");
    for (name, diff) in diffs {
        println!("{} {}", sigil(diff), name);
    }

    let modified = index_filelist.iter().filter(|ie| {
        !ie.is_same_as_tree(&repo)
            .expect("hecked up while checking if files are the same as they are in the tree")
    });

    // TODO: show untracked files
    println!("\nModified files in working tree");
    for f in modified {
        println!("~ {}", f.name);
    }

    Ok(())
}

// -----------------------------------------
// Plumbing Commands
// -----------------------------------------

/// makes a commit of a tree
pub fn commit_tree(id: Id, who: String, message: String) -> Result<()> {
    let repo = Repo::new().context("couldn't find repo")?;
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
    if let Ok(head) = repo.head() {
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
pub fn new_tree(paths: Vec<String>) -> Result<()> {
    let repo = Repo::new().context("failed to find .git")?;
    let paths = paths.iter().map(|p| Path::new(p)).collect::<Vec<&Path>>();
    for &path in &paths {
        // TODO: support handling directories. probably requires thought re:
        // symlinks
        if !path.is_file() {
            return Err(anyhow!("{} is not a file", &path.display()));
        }
    }

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

/// dumps the content of an object in the database for debugging purposes
pub fn catfile(id: &str, output: OutputType) -> Result<()> {
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

/// parses and prints various objects in debug format
pub fn debug(what: args::DebugType) -> Result<()> {
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
        args::DebugType::Test => {
            // a debug entry point
        }
    }
    Ok(())
}

pub fn rev_parse(find_rev: String) -> Result<()> {
    let repo = Repo::new().context("Failed to find the repo")?;
    println!("{}", rev::parse(&find_rev, &repo)?);
    Ok(())
}

/// Like git update-ref if it was really badly coded and evil.
/// Your Repo May Vary.
pub fn update_ref(target: String, new_id: String) -> Result<()> {
    let repo = Repo::new().context("Failed to find the repo")?;
    let new_id = rev::parse(&new_id, &repo)?;
    rev::update_ref(Path::new(&target), &new_id, &repo.root)?;
    Ok(())
}
