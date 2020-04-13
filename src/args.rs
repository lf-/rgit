use clap::{arg_enum, Clap};

#[derive(Clap)]
#[clap(version = "0.0.1", author = "lf")]
pub(crate) struct Opts {
    #[clap(subcommand)]
    pub(crate) subcmd: SubCommand,
}

#[derive(Clap)]
pub(crate) enum SubCommand {
    /// 🐱 dumps the content of an object file with a given ID
    CatFile(CatFile),

    /// 🌳 makes a tree object from the given directory
    NewTree(NewTree),

    /// 🔃🌳 commits a tree object
    CommitTree(CommitTree),

    /// ✨ makes a new repo
    Init,
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
