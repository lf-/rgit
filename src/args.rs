use clap::{arg_enum, Clap};

#[derive(Clap)]
#[clap(version = "0.0.1", author = "lf")]
pub(crate) struct Opts {
    /// How verbose to be when printing output
    #[clap(short = "v", parse(from_occurrences))]
    pub(crate) verbose: usize,

    #[clap(subcommand)]
    pub(crate) subcmd: SubCommand,
}

#[derive(Clap)]
pub(crate) enum SubCommand {
    /// 🐱 dumps the content of an object file with a given ID
    CatFile(CatFile),

    /// 🔃🌳 commits a tree object
    CommitTree(CommitTree),

    /// 🐛 dumps debug info about various files
    Debug(Debug),

    /// ✨ makes a new repo
    Init,

    /// 🌳 makes a tree object from the given file paths
    NewTree(NewTree),
}

// :( this should be pub(crate) but the macro eats it
arg_enum! {
pub enum OutputType {
    Raw,
    Quoted,
    Debug
}
}

#[derive(Clap)]
pub(crate) struct CatFile {
    #[clap(index = 1)]
    pub(crate) git_ref: String,

    #[clap(long, short = "o", required = false, case_insensitive = true,
           default_value = "Raw", possible_values = &OutputType::variants())]
    pub(crate) output: OutputType,
}

#[derive(Clap)]
pub(crate) struct NewTree {
    #[clap(index = 1, multiple = true)]
    pub(crate) paths: Vec<String>,
}

#[derive(Clap)]
pub(crate) struct CommitTree {
    #[clap(index = 1)]
    /// id of the tree object to commit
    pub(crate) id: String,

    #[clap(long, case_insensitive = true)]
    /// Who to commit/author as. Format (remember to quote!):
    /// your_name <email@example.com>
    pub(crate) who: String,

    #[clap(long, case_insensitive = true)]
    /// Commit message
    pub(crate) message: String,
}

#[derive(Clap)]
pub(crate) struct Debug {
    #[clap(index = 1, case_insensitive = true,
        possible_values = &DebugType::variants())]
    /// Which file to debug
    pub(crate) what: DebugType,
}

arg_enum! {
pub enum DebugType {
    Index
}
}
