//! A Git implementation in Rust, mostly for fun
#![feature(is_sorted)]
#![deny(missing_docs, unused_qualifications)]
mod args;
mod commands;
mod diff;
pub mod index;
pub mod num;
pub mod objects;
pub mod tree;
pub mod util;

use anyhow::{Context, Result};
use args::SubCommand;
use clap::Clap;

use crate::objects::Id;

#[macro_use]
extern crate log;

/// The actual main function, wrapped to use results.
fn do_main(opts: args::Opts) -> Result<()> {
    match opts.subcmd {
        SubCommand::Add(a) => commands::add(a.files),
        SubCommand::Commit(c) => commands::commit(c.who, c.message),
        //SubCommand::Diff(d) => commands::diff(d),
        SubCommand::Init => commands::init(),
        SubCommand::Status => commands::status(),
        // plumbing
        SubCommand::CatFile(cf) => commands::catfile(&cf.git_ref, cf.output),
        SubCommand::CommitTree(c) => {
            let id = Id::from(&c.id).context("invalid ID format")?;
            commands::commit_tree(id, c.who, c.message)
        }
        SubCommand::Debug(ty) => commands::debug(ty.what),
        SubCommand::NewTree(m) => commands::new_tree(m.paths),
    }
}

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

    match do_main(opts) {
        Ok(_) => (), // success
        Err(e) => {
            eprintln!("Encountered error: {}", e);
        }
    }
}
