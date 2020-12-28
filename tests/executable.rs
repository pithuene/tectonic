// Copyright 2016-2018 the Tectonic Project
// Licensed under the MIT License.

use lazy_static::lazy_static;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    str,
};
use tempfile::TempDir;

#[path = "util/mod.rs"]
mod util;
use crate::util::{cargo_dir, ensure_plain_format};

lazy_static! {
    static ref TEST_ROOT: PathBuf = {
        util::set_test_root();

        let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        root.push("tests");
        root
    };

    static ref TARGET_RUNNER_WORDS: Vec<String> = {
        // compile-time environment variable from build.rs:
        let target = env!("TARGET").to_owned();
        let mut target = target.replace('-', "_");
        target.make_ascii_uppercase();

        // run-time environment variable check:
        if let Ok(runtext) = env::var(format!("CARGO_TARGET_{}_RUNNER", target)) {
            runtext.split_whitespace().map(|x| x.to_owned()).collect()
        } else {
            vec![]
        }
    };

    // Special coverage-collection mode. This implementation is quite tuned for
    // the Tectonic CI/CD system, so if you're trying to use it manually, expect
    // some rough edges.
    static ref KCOV_WORDS: Vec<String> = {
        if let Ok(runtext) = env::var("TECTONIC_EXETEST_KCOV_RUNNER") {
            runtext.split_whitespace().map(|x| x.to_owned()).collect()
        } else {
            vec![]
        }
    };
}

fn get_plain_format_arg() -> String {
    util::set_test_root();
    let path = ensure_plain_format().expect("couldn't write format file");
    format!("--format={}", path.display())
}

/// Note the special sauce here — we set the magic environment variable that
/// tells the Tectonic binary to go into "test mode" and use local test
/// assets, rather than an actual network bundle.
fn prep_tectonic(cwd: &Path, args: &[&str]) -> Command {
    let tectonic = cargo_dir()
        .join("tectonic")
        .with_extension(env::consts::EXE_EXTENSION);

    if fs::metadata(&tectonic).is_err() {
        panic!(
            "tectonic binary not found at {:?}. Do you need to run `cargo build`?",
            tectonic
        )
    }
    println!("using tectonic binary at {:?}", tectonic);
    println!("using cwd {:?}", cwd);

    // We may need to wrap the Tectonic invocation. If we're cross-compiling, we
    // might need to use something like QEMU to actually be able to run the
    // executable. If we're collecting code coverage information with kcov, we
    // need to wrap the invocation with that program.
    let mut command = if TARGET_RUNNER_WORDS.len() > 0 {
        let mut cmd = Command::new(&TARGET_RUNNER_WORDS[0]);
        cmd.args(&TARGET_RUNNER_WORDS[1..]).arg(tectonic);
        cmd
    } else if KCOV_WORDS.len() > 0 {
        let mut cmd = Command::new(&KCOV_WORDS[0]);
        cmd.args(&KCOV_WORDS[1..]);

        // Give kcov a directory into which to put its output. We use
        // mktemp-like functionality to automatically create such directories
        // uniquely so that we don't have to manually bookkeep. This does mean
        // that successive runs will build up new data directories indefinitely.
        let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        root.push("target");
        root.push("cov");
        root.push("exetest.");
        let tempdir = tempfile::Builder::new().prefix(&root).tempdir().unwrap();
        let tempdir = tempdir.into_path();
        cmd.arg(tempdir);

        cmd.arg(tectonic);
        cmd
    } else {
        Command::new(tectonic)
    };

    command.args(args).current_dir(cwd).env(
        tectonic::test_util::TEST_ROOT_ENV_VAR,
        TEST_ROOT.as_os_str(),
    );
    command
}

fn run_tectonic(cwd: &Path, args: &[&str]) -> Output {
    let mut command = prep_tectonic(cwd, args);
    println!("running {:?}", command);
    command.output().expect("tectonic failed to start")
}

fn run_tectonic_with_stdin(cwd: &Path, args: &[&str], stdin: &str) -> Output {
    let mut command = prep_tectonic(cwd, args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    println!("running {:?}", command);
    let mut child = command.spawn().expect("tectonic failed to start");
    write!(child.stdin.as_mut().unwrap(), "{}", stdin)
        .expect("failed to send data to tectonic subprocess");
    child
        .wait_with_output()
        .expect("failed to wait on tectonic subprocess")
}

fn setup_and_copy_files(files: &[&str]) -> TempDir {
    let tempdir = tempfile::Builder::new()
        .prefix("tectonic_executable_test")
        .tempdir()
        .unwrap();

    // `cargo kcov` (0.5.2) does not set this variable:
    let executable_test_dir = if let Some(v) = env::var_os("CARGO_MANIFEST_DIR") {
        PathBuf::from(v)
    } else {
        PathBuf::new()
    }
    .join("tests/executable");

    for file in files {
        // Create parent directories, if the file is not at the root of `tests/executable/`
        let file_path = PathBuf::from(file);
        let parent_dir = file_path.parent().unwrap();
        let mut dirbuilder = fs::DirBuilder::new();
        dirbuilder.recursive(true);
        dirbuilder.create(tempdir.path().join(parent_dir)).unwrap();

        fs::copy(executable_test_dir.join(file), tempdir.path().join(file)).unwrap();
    }

    tempdir
}

fn success_or_panic(output: Output) {
    if output.status.success() {
        println!("status: {}", output.status);
        println!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        println!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    } else {
        panic!(
            "Command exited badly:\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn error_or_panic(output: Output) {
    if !output.status.success() {
        println!("status: {}", output.status);
        println!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        println!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    } else {
        panic!(
            "Command should have failed but didn't:\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn check_file(tempdir: &TempDir, rest: &str) {
    let mut p = tempdir.path().to_owned();
    p.push(rest);

    if !p.is_file() {
        panic!(
            "file \"{}\" should have been created but wasn\'t",
            p.to_string_lossy()
        );
    }
}

/* Keep tests alphabetized */

#[test]
fn bad_chatter_1() {
    let output = run_tectonic(&PathBuf::from("."), &["-", "--chatter=reticent"]);
    error_or_panic(output);
}

#[test]
fn bad_input_path_1() {
    let output = run_tectonic(&PathBuf::from("."), &["/"]);
    error_or_panic(output);
}

#[test]
fn bad_input_path_2() {
    let output = run_tectonic(&PathBuf::from("."), &["somedir/.."]);
    error_or_panic(output);
}

#[test]
fn bad_outfmt_1() {
    let output = run_tectonic(&PathBuf::from("."), &["-", "--outfmt=dd"]);
    error_or_panic(output);
}

#[test]
fn help_flag() {
    let output = run_tectonic(&PathBuf::from("."), &["-h"]);
    success_or_panic(output);
}

#[test]
fn keep_logs_on_error() {
    // No input files here, but output files are created.
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&[]);
    let output = run_tectonic_with_stdin(
        tempdir.path(),
        &[&fmt_arg, "-", "--keep-logs"],
        "no end to this file",
    );
    error_or_panic(output);

    let mut log = String::new();
    File::open(tempdir.path().join("texput.log"))
        .expect("`texput.log` not found")
        .read_to_string(&mut log)
        .expect("Cannot read `texput.log`");

    assert!(log.contains(r"job aborted, no legal \end found"));
}

#[test]
fn no_color_option() {
    // No input files here, but output files are created.
    let fmt_arg = get_plain_format_arg();

    let tempdir = setup_and_copy_files(&[]);
    let output_nocolor = run_tectonic_with_stdin(
        tempdir.path(),
        &[&fmt_arg, "-", "--color=never"],
        "no end to this file",
    );

    // Output is not a terminal, so these two should be the same
    let tempdir = setup_and_copy_files(&[]);
    let output_autocolor = run_tectonic_with_stdin(
        tempdir.path(),
        &[&fmt_arg, "-", "--color=auto"],
        "no end to this file",
    );

    assert_eq!(output_nocolor, output_autocolor);

    error_or_panic(output_nocolor);
    error_or_panic(output_autocolor);
}

#[test]
fn outdir_option() {
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&["subdirectory/content/1.tex"]);

    let output = run_tectonic(
        tempdir.path(),
        &[
            &fmt_arg,
            "subdirectory/content/1.tex",
            "--outdir=subdirectory",
        ],
    );
    success_or_panic(output);
    check_file(&tempdir, "subdirectory/1.pdf");
}

#[test]
#[should_panic]
// panic unwinding broken: https://github.com/rust-embedded/cross/issues/343
#[cfg(not(all(target_arch = "arm", target_env = "musl")))]
fn outdir_option_bad() {
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&["subdirectory/content/1.tex"]);

    let output = run_tectonic(
        tempdir.path(),
        &[
            &fmt_arg,
            "subdirectory/content/1.tex",
            "--outdir=subdirectory/non_existent",
        ],
    );
    success_or_panic(output);
}

#[test]
#[should_panic]
// panic unwinding broken: https://github.com/rust-embedded/cross/issues/343
#[cfg(not(all(target_arch = "arm", target_env = "musl")))]
fn outdir_option_is_file() {
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&["test space.tex", "subdirectory/content/1.tex"]);

    let output = run_tectonic(
        tempdir.path(),
        &[
            &fmt_arg,
            "subdirectory/content/1.tex",
            "--outdir=test space.tex",
        ],
    );
    success_or_panic(output);
}

#[test] // GitHub #31
fn relative_include() {
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&[
        "subdirectory/relative_include.tex",
        "subdirectory/content/1.tex",
    ]);

    let output = run_tectonic(
        tempdir.path(),
        &[&fmt_arg, "subdirectory/relative_include.tex"],
    );
    success_or_panic(output);
    check_file(&tempdir, "subdirectory/relative_include.pdf");
}

// Regression #36
#[test]
fn space_in_filename() {
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&["test space.tex"]);

    let output = run_tectonic(tempdir.path(), &[&fmt_arg, "test space.tex"]);
    success_or_panic(output);
}

#[test]
fn stdin_content() {
    // No input files here, but output files are created.
    let fmt_arg = get_plain_format_arg();
    let tempdir = setup_and_copy_files(&[]);
    let output = run_tectonic_with_stdin(
        tempdir.path(),
        &[&fmt_arg, "-"],
        "Standard input content.\\bye",
    );
    success_or_panic(output);
}

#[test]
fn v2_new_build() {
    util::set_test_root();

    let tempdir = setup_and_copy_files(&[]);
    let mut temppath = tempdir.path().to_owned();
    let output = run_tectonic(&temppath, &["-X", "new", "doc"]);
    success_or_panic(output);

    temppath.push("doc");

    // To run a build in our test setup, we can only use plain TeX. So, jankily
    // change the format ...

    {
        let mut toml_path = temppath.clone();
        toml_path.push("Tectonic.toml");
        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(toml_path)
            .unwrap();
        writeln!(file, "tex_format = 'plain'").unwrap();
    }

    // ... and write some files that are plain TeX.

    {
        let mut path = temppath.clone();
        path.push("src");

        {
            path.push("_preamble.tex");
            let mut file = File::create(&path).unwrap();
            writeln!(file).unwrap();
            path.pop();
        }

        {
            path.push("_postamble.tex");
            let mut file = File::create(&path).unwrap();
            writeln!(file, "\\end").unwrap();
            path.pop();
        }
    }

    // Now we can build.

    let output = run_tectonic(&temppath, &["-X", "build"]);
    success_or_panic(output);
}
