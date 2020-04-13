mod args;
mod commands;
mod num;
mod objects;

use args::SubCommand;
use clap::Clap;

fn main() {
    let opts = args::Opts::parse();

    match opts.subcmd {
        SubCommand::CatFile(cf) => {
            commands::catfile(&cf.git_ref, cf.output).unwrap();
        }
        SubCommand::CommitTree(c) => {
            commands::commit_tree(c.id, c.who, c.message).unwrap();
        }
        SubCommand::NewTree(m) => {
            commands::new_tree(m.paths).unwrap();
        }
        SubCommand::Init => {
            commands::init().unwrap();
        }
    }
}
