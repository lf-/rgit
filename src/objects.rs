use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset};
use flate2::bufread::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha1::{Digest, Sha1};
use std::env;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::str;

use crate::num;

fn open_compressed(path: &Path) -> Result<impl Read> {
    let file = fs::File::open(path).context("Failed to open compressed file")?;
    let decoder = ZlibDecoder::new(BufReader::new(file));
    Ok(decoder)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Id([u8; 20]);

pub struct Repo {
    /// path to the root of the .git directory
    root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameEntry {
    pub name: String,
    pub email: String,
    pub time: DateTime<FixedOffset>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Commit {
    pub tree: Id,
    pub parents: Vec<Id>,
    pub author: NameEntry,
    pub committer: NameEntry,
    pub message: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct File {
    pub mode: u32,
    // opinion: this is UTF-8 encoded. cgit doesn't care however
    pub name: String,
    pub id: Id,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tree {
    pub files: Vec<File>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Blob {
    content: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Object {
    Tree(Tree),
    Blob(Blob),
    Commit(Commit),
}

impl Repo {
    /// Makes a new repo, trying to find a .git directory in children
    pub fn new() -> Option<Repo> {
        let cwd = env::current_dir().ok()?;
        for dir in cwd.as_path().ancestors() {
            let dotgit = dir.join(".git");
            if dotgit.is_dir() {
                return Some(Repo { root: dotgit });
            }
        }
        None
    }

    /// Initializes a repo at `root/.git`
    pub fn init(tree_root: &Path) -> Result<Repo> {
        let root = tree_root.join(".git");
        fs::create_dir(&root)?;

        fs::create_dir(root.join("refs"))?;
        fs::create_dir(root.join("refs/heads"))?;
        fs::create_dir(root.join("objects"))?;

        fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(root.join("HEAD"))
            .context("failed creating HEAD")?
            .write_all(b"ref: refs/heads/master")?;
        Ok(Repo { root: root.into() })
    }

    fn path_for_object(&self, id: &Id) -> PathBuf {
        let id = format!("{}", id);
        let mut path = self.root.clone();
        path.push("objects");
        path.push(&id[..2]);
        path.push(&id[2..]);
        path
    }

    pub fn open_object(&self, id: &Id) -> Result<impl Read> {
        open_compressed(&self.path_for_object(id))
    }

    fn head_path(&self) -> PathBuf {
        self.root.as_path().join("HEAD")
    }

    /// Gets the current value of the HEAD pointer
    /// It can validly be None
    pub fn head(&self) -> Result<Option<Id>> {
        let id_s = fs::read_to_string(self.head_path())?;
        Ok(Id::from(id_s.trim()))
    }

    pub fn set_head(&self, new_head: &Id) -> Result<()> {
        let id_s = format!("{}", new_head);
        fs::write(self.head_path(), id_s).context("hecked up setting head")
    }

    /// Checks if this Id is in the database
    pub fn has_id(&self, id: &Id) -> bool {
        self.path_for_object(id).exists()
    }

    /// Get the root of the repo's tree
    /// I'm pretty sure there's something with bare repos or multiple trees that
    /// we're not supporting here but I don't know what it is and enjoy living in
    /// blissful ignorance
    pub fn tree_root(&self) -> PathBuf {
        self.root
            .parent()
            .expect("your .git is at the root of your fs?")
            .to_path_buf()
    }
}

#[test]
fn test_path_for_object() {
    let repo = Repo {
        root: "/path/to/root/.git".into(),
    };
    assert_eq!(
        repo.path_for_object(&Id::from("0096cfbd9d1001af3731d9ab5de79450fe031719").unwrap()),
        Path::new("/path/to/root/.git/objects/00/96cfbd9d1001af3731d9ab5de79450fe031719")
    )
}

impl NameEntry {
    pub fn from(s: &str) -> Option<NameEntry> {
        // format: NAME <EMAIL> 12345 -0900
        let mut iter = s.rsplitn(3, ' ');
        let offs = iter.next()?;
        let timestamp = iter.next()?;

        let time =
            DateTime::<FixedOffset>::parse_from_str(&(timestamp.to_owned() + " " + offs), "%s %z")
                .ok()?;

        Self::with_time(iter.next()?, time)
    }

    pub fn with_time(s: &str, time: DateTime<FixedOffset>) -> Option<NameEntry> {
        let mut iter = s.rsplitn(2, ' ');

        let email_part = iter.next()?;
        // chop off brackets
        let email = &email_part[1..email_part.len() - 1];
        let name = iter.next()?;

        Some(NameEntry {
            name: name.to_owned(),
            email: email.to_owned(),
            time,
        })
    }

    fn encode(&self) -> Vec<u8> {
        let time = self.time.format("%s %z");
        format!("{} <{}> {}", self.name, self.email, time).into_bytes()
    }
}

impl fmt::Display for NameEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        // Wed Apr 8 17:10:37 2020 -0700
        let time = self.time.format("%a %b %-d %Y %H:%M:%S %z");
        write!(formatter, "{} <{}> {}", self.name, self.email, time)
    }
}

#[test]
fn test_load_parse_name_entry() {
    let entry = NameEntry {
        name: "two names".to_owned(),
        email: "email@example.com".to_owned(),
        time: DateTime::parse_from_rfc3339("2000-01-01T00:00:00-01:30").unwrap(),
    };
    let entry_s = "two names <email@example.com> 946690200 -0130";
    assert_eq!(NameEntry::from(entry_s).unwrap(), entry);
    assert_eq!(
        format!("{}", entry),
        "two names <email@example.com> Sat Jan 1 2000 00:00:00 -0130"
    );
}

impl Id {
    /// Decode an ID from hex representation
    pub fn from(s: &str) -> Option<Id> {
        let decoded = num::parse_hex(s.as_bytes())?;

        // check length here to avoid panic in copy_from_slice
        if decoded.len() != 20 {
            return None;
        }
        let mut id_inner = [0; 20];
        id_inner.copy_from_slice(&decoded);
        Some(Id(id_inner))
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for ch in &self.0 {
            write!(f, "{:02x}", ch)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Id({})", self)
    }
}

#[test]
fn test_id_as_hex() {
    assert_eq!(
        format!("{}", Id(*b"abababababababababac")),
        "6162616261626162616261626162616261626163"
    );
    // checks for regression on a bug where there is incorrect padding on encoded bytes
    assert_eq!(
        format!(
            "{}",
            Id::from("94546d68dc6002b85cc2d7df077c7c6bb080abb0").unwrap()
        ),
        "94546d68dc6002b85cc2d7df077c7c6bb080abb0"
    )
}

impl Blob {
    pub fn from(content: &[u8]) -> Blob {
        // it is probably a bad idea to copy the full file content into memory
        // for no reason
        Blob {
            content: content.to_vec(),
        }
    }

    pub fn new_from_disk(path: &Path) -> Result<Blob> {
        Ok(Blob {
            content: fs::read(path)?,
        })
    }

    fn encode(&self) -> Vec<u8> {
        self.content.clone()
    }
}

impl File {
    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(format!("{:o}", self.mode).into_bytes());
        v.push(b' ');
        v.extend(self.name.as_bytes());
        v.push(0x00);
        v.extend(&self.id.0);
        v
    }
}

#[test]
fn test_file_encoding() {
    let f = File {
        name: "d".to_string(),
        mode: 0o40000,
        id: Id(*b"??\x1d_tbl?/?}7?Ar??\x1c\x7f?"),
    };
    assert_eq!(f.encode(), b"40000 d\x00??\x1d_tbl?/?}7?Ar??\x1c\x7f?");
}

impl Tree {
    fn from(content: &[u8]) -> Result<Tree> {
        // each record is:
        // <octal mode> <name>\x00<20 byte sha1 hash in binary>
        let mut rest = content;
        let mut files = Vec::new();

        while rest.len() > 0 {
            // <octal mode><SPACE><...>
            let mut split = rest.splitn(2, |&b| b == ' ' as u8);
            let mode = num::parse_octal(split.next().context("corrupt Tree records")?)
                .context("corrupt Tree record mode")?;
            rest = split.next().context("corrupt Tree structure")?;

            // <name><0x00><...>
            let mut split = rest.splitn(2, |&b| b == 0x00);
            let name = split
                .next()
                .context("corrupt Tree structure, missing null")?;
            rest = split.next().context("corrupt Tree structure")?;

            // <hash><...>
            let mut hash = [0u8; 20];
            hash.clone_from_slice(&rest[..20]);

            files.push(File {
                name: String::from(str::from_utf8(name).context("filename not UTF-8 compliant")?),
                id: Id(hash),
                mode,
            });
            rest = &rest[20..];
        }
        Ok(Tree { files })
    }

    fn encode(&self) -> Vec<u8> {
        // there is probably a sin here: we should be using iterators somehow
        let mut v = Vec::new();
        for f in &self.files {
            v.extend(f.encode());
        }
        v
    }
}

#[test]
fn test_tree_parsing() {
    let tree = Tree::from(
        b"40000 d\x00??\x1d_tbl?/?}7?Ar??\x1c\x7f?100644 \
        hello.txt\x00?\x016%\x03\x0b???\x06?V?\x7f????FJ",
    );
    assert_eq!(
        tree.unwrap(),
        Tree {
            files: vec![
                File {
                    name: "d".to_string(),
                    mode: 0o40000,
                    id: Id(*b"??\x1d_tbl?/?}7?Ar??\x1c\x7f?"),
                },
                File {
                    name: "hello.txt".to_string(),
                    mode: 0o100644,
                    id: Id(*b"?\x016%\x03\x0b???\x06?V?\x7f????FJ"),
                }
            ]
        }
    )
}

impl Commit {
    pub fn from(content: &[u8]) -> Result<Commit> {
        let content = content.to_vec();
        let mut slice = content.as_slice();

        let mut buf = String::new();
        let mut tree = None;
        let mut parents = Vec::new();
        let mut committer = None;
        let mut author = None;

        loop {
            buf.clear();
            let res = slice.read_line(&mut buf);
            match res {
                // we should never hit EOF since we are reading the header of
                // the commit message
                Ok(0) => return Err(anyhow!("hit unexpected EOF reading commit metadata")),
                Ok(_) => {
                    let trimmed = buf.trim_end_matches(|c| c == '\n' || c == '\r');

                    if trimmed == "" {
                        // end of header block. Commit message begins below.
                        // We're done here.
                        break;
                    }

                    let mut iter = trimmed.splitn(2, ' ');
                    let typ = iter
                        .next()
                        .context("unexpected empty line reading commit metadata")?;
                    let rest = iter
                        .next()
                        .context("got confused reading commit metadata")?;

                    match typ {
                        // this pattern of Some(x?) looks dumb but I want to
                        // ensure that the parse error gets reported as such
                        // rather than the missing error
                        "tree" => tree= Some(Id::from(rest).context("tree was not an id")?),
                        "parent" => parents.push(Id::from(rest).context("parent was not an id")?),
                        "author" => author = Some(NameEntry::from(rest).context("failed to parse author")?),
                        "committer" => committer = Some(NameEntry::from(rest).context("failed to parse committer")?),
                        _ => eprintln!("found something not seen before in commit metadata, type {:?} rest {:?}", typ, rest),
                    }
                }
                Err(e) => return Err(e).context("read error reading commit metadata"),
            }
        }
        Ok(Commit {
            tree: tree.context("tree missing when parsing commit header")?,
            author: author.context("author missing when parsing commit header")?,
            committer: committer.context("committer missing when parsing commit header")?,
            message: str::from_utf8(&slice)?.to_string(),
            parents,
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"tree ");
        v.extend(format!("{}", self.tree).as_bytes());
        for parent in &self.parents {
            v.extend(b"\nparent ");
            v.extend(format!("{}", parent).as_bytes());
        }
        v.extend(b"\nauthor ");
        v.extend(self.author.encode());
        v.extend(b"\ncommitter ");
        v.extend(self.committer.encode());
        v.extend(b"\n\n");
        v.extend(self.message.as_bytes());
        v
    }
}

#[test]
fn test_commit_parse_encode() {
    let commit = b"tree 94546d68dc6002b85cc2d7df077c7c6bb080abb0\n\
                   parent d55912e4475329fde95d52d619abd413e4001d68\n\
                   parent d30826db9da3aebc9ab7fc095dd964920fc299bf\n\
                   author lf- <lf-@users.noreply.github.com> 1586391037 -0700\n\
                   committer lf- <lf-@users.noreply.github.com> 1586391037 -0700\n\n\
                   Merge branch \'branch2\'\n"
        .to_vec();
    let decoded = Commit {
        tree: Id::from("94546d68dc6002b85cc2d7df077c7c6bb080abb0").unwrap(),
        parents: vec![
            Id::from("d55912e4475329fde95d52d619abd413e4001d68").unwrap(),
            Id::from("d30826db9da3aebc9ab7fc095dd964920fc299bf").unwrap(),
        ],

        author: NameEntry::from("lf- <lf-@users.noreply.github.com> 1586391037 -0700").unwrap(),
        committer: NameEntry::from("lf- <lf-@users.noreply.github.com> 1586391037 -0700").unwrap(),
        message: "Merge branch \'branch2\'\n".to_string(),
    };
    assert_eq!(Commit::from(&commit).unwrap(), decoded);
    assert_eq!(decoded.encode(), commit);
}

impl Object {
    /// Opens an existing object on disk and parses it into an Object
    /// structure
    pub fn open(repo: &Repo, id: &Id) -> Result<Object> {
        let mut stream = repo
            .open_object(&id)
            .context(format!("Failed to open object {} on disk", id))?;

        let mut buf = Default::default();

        stream.read_to_end(&mut buf).context(format!(
            "Failed reading decompressed stream from object {}",
            id
        ))?;
        // question mark operator *inside* an Ok is possibly evil
        Ok(Self::parse(buf).context(format!("Failed to parse object {}", id))?)
    }

    fn parse(buf: Vec<u8>) -> Result<Object> {
        let mut split = buf.splitn(2, |&e| e == 0x00);
        let header = split.next().context(format!("Malformed object file"))?;

        let content = split
            .next()
            .context(format!("Missing null termination after object size"))?;

        let objtype = str::from_utf8(
            header
                .split(|&e| e == ' ' as u8)
                .next()
                .context("Failed to parse object type")?,
        )?;

        Ok(match objtype {
            "tree" => Object::Tree(Tree::from(content)?),
            "blob" => Object::Blob(Blob::from(content)),
            "commit" => Object::Commit(Commit::from(content)?),
            _ => return Err(anyhow!("unsupported object type {}", objtype)),
        })
    }

    pub fn store(&self, repo: &Repo) -> Result<Id> {
        let (id, content) = self.prepare_store();

        if repo.has_id(&id) {
            // don't store IDs that already exist
            return Ok(id);
        }

        let path = repo.path_for_object(&id);
        fs::create_dir_all(
            path.as_path()
                .parent()
                .context("unexpected filesystem boundary found in your .git directory")?,
        )?;

        fs::write(&path, content)?;
        Ok(id)
    }

    fn prepare_store(&self) -> (Id, Vec<u8>) {
        let (typ, encoded) = match self {
            Object::Tree(t) => ("tree", t.encode()),
            Object::Blob(b) => ("blob", b.encode()),
            Object::Commit(c) => ("commit", c.encode()),
        };

        let size = encoded.len();
        let mut to_store = Vec::new();
        to_store.extend(typ.as_bytes());
        to_store.push(b' ');
        to_store.extend(format!("{}", size).as_bytes());
        to_store.push(0x00);
        to_store.extend(encoded);

        let mut hasher = Sha1::new();
        hasher.input(&to_store);
        let id = Id(hasher.result().into());

        let mut squished = Vec::new();
        let mut squisher = ZlibEncoder::new(&mut squished, Compression::best());
        squisher
            .write_all(&to_store[..])
            .expect("writing to in-memory compression stream failed. wat.");
        squisher
            .finish()
            .expect("compression finalization failed. wat");

        (id, squished)
    }
}

#[test]
fn test_object_encoding() {
    let decoded = Commit {
        tree: Id::from("94546d68dc6002b85cc2d7df077c7c6bb080abb0").unwrap(),
        parents: vec![
            Id::from("d55912e4475329fde95d52d619abd413e4001d68").unwrap(),
            Id::from("d30826db9da3aebc9ab7fc095dd964920fc299bf").unwrap(),
        ],

        author: NameEntry::from("lf- <lf-@users.noreply.github.com> 1586391037 -0700").unwrap(),
        committer: NameEntry::from("lf- <lf-@users.noreply.github.com> 1586391037 -0700").unwrap(),
        message: "Merge branch \'branch2\'\n".to_string(),
    };
    let (id, squished_content) = Object::Commit(decoded).prepare_store();

    let mut unsquisher = flate2::read::ZlibDecoder::new(&squished_content[..]);

    let mut content = Vec::new();
    unsquisher.read_to_end(&mut content).unwrap();
    assert_eq!(
        id,
        Id::from("b1ea81dd8e9465cd9d2753d4bb3652d13c78312d").unwrap()
    );
    assert_eq!(
        content,
        b"commit 287\x00tree 94546d68dc6002b85cc2d7df077c7c6bb080abb0\n\
        parent d55912e4475329fde95d52d619abd413e4001d68\n\
        parent d30826db9da3aebc9ab7fc095dd964920fc299bf\n\
        author lf- <lf-@users.noreply.github.com> 1586391037 -0700\n\
        committer lf- <lf-@users.noreply.github.com> 1586391037 -0700\n\nMerge branch 'branch2'\n"
            .to_vec()
    );
}

#[test]
fn test_object_parsing() {
    // tree
    let tree = b"tree 102\x0040000 d\x00??\x1d_tbl?/?}7?Ar??\x1c\x7f?100644 \
        hello.txt\x00?\x016%\x03\x0b???\x06?V?\x7f????FJ100644 \
        world.txt\x00?b??\x10t+??$\x1cY$??+\\\x01?q";
    assert_eq!(
        Object::parse(tree.to_vec()).unwrap(),
        Object::Tree(Tree {
            files: vec![
                File {
                    name: "d".to_string(),
                    mode: 0o40000,
                    id: Id(*b"??\x1d_tbl?/?}7?Ar??\x1c\x7f?"),
                },
                File {
                    name: "hello.txt".to_string(),
                    mode: 0o100644,
                    id: Id(*b"?\x016%\x03\x0b???\x06?V?\x7f????FJ"),
                },
                File {
                    name: "world.txt".to_string(),
                    mode: 0o100644,
                    id: Id(*b"?b??\x10t+??$\x1cY$??+\\\x01?q"),
                }
            ]
        })
    );

    // blob
    let blob = b"blob 6\x00hello";
    assert_eq!(
        Object::parse(blob.to_vec()).unwrap(),
        Object::Blob(Blob {
            content: b"hello".to_vec(),
        })
    );

    // unsupported
    let sadface = b"sadface 1\x00";
    assert!(Object::parse(sadface.to_vec()).is_err());
}
