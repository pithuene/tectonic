// src/bin/tectonic.rs -- Command-line driver for the Tectonic engine.
// Copyright 2016-2020 the Tectonic Project
// Licensed under the MIT License.

use std::{env, process, str::FromStr};
use structopt::StructOpt;

use tectonic::{
    config::PersistentConfig,
    status::plain::PlainStatusBackend,
    status::termcolor::TermcolorStatusBackend,
    status::{ChatterLevel, StatusBackend},
    tt_note,
};

mod compile;

#[cfg(feature = "serialization")]
mod v2cli;

// Defused V2 support if serialization is unavailable.
#[cfg(not(feature = "serialization"))]
mod v2cli {
    use std::{ffi::OsString, process};

    pub fn v2_main(_effective_args: &[OsString]) {
        eprintln!(
            "fatal error: the \"V2\" Tectonic CLI requires the code to have been built \
            with the \"serialization\" Cargo feature active. This one wasn't."
        );
        process::exit(1);
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "Tectonic", about = "Process a (La)TeX document")]
struct CliOptions {
    /// How much chatter to print when running
    #[structopt(long = "chatter", short, name = "level", default_value = "default", possible_values(&["default", "minimal"]))]
    chatter_level: String,

    /// Enable/disable colorful log output.
    #[structopt(long = "color", name = "when", default_value = "auto", possible_values(&["always", "auto", "never"]))]
    cli_color: String,

    #[structopt(flatten)]
    compile: compile::CompileOptions,
}

fn main() {
    // Migration to the "cargo-style" command-line interface. If the first
    // argument is `-X`, or argv[0] contains `nextonic`, we activate the
    // alternative operation mode. Once this experimental mode is working OK,
    // we'll start printing a message telling people to prefer the `-X` option
    // and optionally accept a leading `-Y` option for the "classic"
    // ("rustc"-style, current) interface. After that's been in place for a
    // while, well swap the defaults and make `-Y` required if you want to use
    // the classic interface. Finally, we'll remove it altogether.

    let mut v2cli_enabled = false;
    let os_args: Vec<_> = env::args_os().collect();
    let mut v2cli_arg_idx = 1;

    if !os_args.is_empty() && os_args[0].to_str().map(|s| s.contains("nextonic")) == Some(true) {
        v2cli_enabled = true;
    } else if os_args.len() > 1 && os_args[1] == "-X" {
        v2cli_enabled = true;
        v2cli_arg_idx = 2;
    }

    if v2cli_enabled {
        v2cli::v2_main(&os_args[v2cli_arg_idx..]);
        return;
    }

    // OK, we're still using the "rustc-style" CLI. Proceed here.

    let args = CliOptions::from_args();

    // The Tectonic crate comes with a hidden internal "test mode" that forces
    // it to use a specified set of local files, rather than going to the
    // bundle -- this makes it so that we can run tests without having to go
    // to the network or touch the current user's cache.
    //
    // This mode is activated by setting a special environment variable. The
    // following call checks for it and activates the mode if necessary. Note
    // that this test infrastructure is lightweight, so I don't think it's a
    // big deal to include the code in the final executable artifacts we
    // distribute.

    tectonic::test_util::maybe_activate_test_mode();

    // I want the CLI program to take as little configuration as possible, but
    // we do need to at least provide a mechanism for storing the default
    // bundle.

    let config = match PersistentConfig::open(false) {
        Ok(c) => c,
        Err(ref e) => {
            // Uhoh, we couldn't get the configuration. Our main
            // error-printing code requires a 'status' object, which we don't
            // have yet. If we can't even load the config we might really be
            // in trouble, so it seems safest to keep things simple anyway and
            // just use bare stderr without colorization.
            e.dump_uncolorized();
            process::exit(1);
        }
    };

    // Set up colorized output. This comes after the config because you could
    // imagine wanting to be able to configure the colorization (which is
    // something I'd be relatively OK with since it'd only affect the progam
    // UI, not the processing results).

    let chatter_level = ChatterLevel::from_str(&args.chatter_level).unwrap();
    let use_cli_color = match &*args.cli_color {
        "always" => true,
        "auto" => atty::is(atty::Stream::Stdout),
        "never" => false,
        _ => unreachable!(),
    };

    let mut status = if use_cli_color {
        Box::new(TermcolorStatusBackend::new(chatter_level)) as Box<dyn StatusBackend>
    } else {
        Box::new(PlainStatusBackend::new(chatter_level)) as Box<dyn StatusBackend>
    };

    // For now ...

    tt_note!(
        status,
        "this is a BETA release; ask questions and report bugs at https://tectonic.newton.cx/"
    );

    // Now that we've got colorized output, pass off to the inner function ...
    // all so that we can print out the word "error:" in red. This code
    // parallels various bits of the `error_chain` crate.

    if let Err(ref e) = args.compile.execute(config, &mut *status) {
        status.report_error(e);
        process::exit(1)
    }
}
