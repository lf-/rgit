use clap::{arg_enum, Clap};

#[derive(Clap)]
#[clap(version = "0.0.1", author = "lf")]
pub struct Opts {
    /// How verbose to be when printing output
    #[clap(short = "v", parse(from_occurrences))]
    pub verbose: usize,

    #[clap(subcommand)]
    pub subcmd: SubCommand,
}

#[derive(Clap)]
pub enum SubCommand {
    /// ➕ adds the given files or directories (recurses!) to the repo
    Add(Add),

    /// 🔃 commits the tree state in the index
    Commit(Commit),

    /// 🆎 diffs blobs and commits
    //Diff(Diff),

    /// ✨ makes a new repo
    Init,

    /// ❓ queries the status of the index vs HEAD and the working tree
    Status,

    // ----- Plumbing -----
    /// 🐱 dumps the content of an object file with a given ID
    CatFile(CatFile),

    /// 🔃🌳 commits a tree object
    CommitTree(CommitTree),

    /// 🐛 dumps debug info about various files
    Debug(Debug),

    /// 🌳 makes a tree object from the given file paths
    NewTree(NewTree),

    /// 🔎 matches the given reference to an id
    RevParse(RevParse),
}

#[derive(Clap)]
pub struct Add {
    /// Files to add to the repo
    #[clap(index = 1, multiple = true)]
    pub files: Vec<String>,
}

// :( this should be pub but the macro eats it
arg_enum! {
pub enum OutputType {
    Raw,
    Quoted,
    Debug
}
}

#[derive(Clap)]
pub struct CatFile {
    #[clap(index = 1)]
    pub git_ref: String,

    #[clap(long, short = "o", required = false, case_insensitive = true,
           default_value = "Raw", possible_values = &OutputType::variants())]
    pub output: OutputType,
}

#[derive(Clap)]
pub struct Commit {
    #[clap(long, case_insensitive = true)]
    /// Who to commit/author as. Format (remember to quote!):
    /// your_name <email@example.com>
    pub who: String,

    #[clap(long, short = "m", case_insensitive = true)]
    /// Commit message
    pub message: String,
}

#[derive(Clap)]
pub struct NewTree {
    /// Paths to add to the new tree
    #[clap(index = 1, multiple = true)]
    pub paths: Vec<String>,
}

#[derive(Clap)]
pub struct CommitTree {
    #[clap(index = 1)]
    /// id of the tree object to commit
    pub id: String,

    #[clap(long, case_insensitive = true)]
    /// Who to commit/author as. Format (remember to quote!):
    /// your_name <email@example.com>
    pub who: String,

    #[clap(long, case_insensitive = true)]
    /// Commit message
    pub message: String,
}

#[derive(Clap)]
pub struct Debug {
    #[clap(index = 1, case_insensitive = true,
        possible_values = &DebugType::variants())]
    /// Which file to debug
    pub what: DebugType,
}

arg_enum! {
pub enum DebugType {
    // Dump debug info on the index
    Index,
    // Run a testing entry point
    Test
}
}

#[derive(Clap)]
pub struct RevParse {
    /// Revision to find
    #[clap(index = 1)]
    pub rev: String,
}
