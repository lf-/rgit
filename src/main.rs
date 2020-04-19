mod args;
mod commands;
mod index;
mod num;
mod objects;
mod util;

use args::SubCommand;
use clap::Clap;

#[macro_use]
extern crate log;

fn main() {
    let opts = args::Opts::parse();

    const INFO: usize = 2;
    // always print INFO messages
    stderrlog::new()
        .module(module_path!())
        .verbosity(opts.verbose + INFO)
        .timestamp(stderrlog::Timestamp::Off)
        .init()
        .unwrap();

    match opts.subcmd {
        SubCommand::CatFile(cf) => {
            commands::catfile(&cf.git_ref, cf.output).unwrap();
        }
        SubCommand::CommitTree(c) => {
            commands::commit_tree(c.id, c.who, c.message).unwrap();
        }
        SubCommand::Debug(ty) => {
            commands::debug(ty.what).unwrap();
        }
        SubCommand::Init => {
            commands::init().unwrap();
        }
        SubCommand::NewTree(m) => {
            commands::new_tree(m.paths).unwrap();
        }
    }
}
