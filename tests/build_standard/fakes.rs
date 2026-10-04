//! Fixtures for the tests that run `make lint` against fake tools.
//!
//! Every run gets a scratch directory under `CARGO_TARGET_TMPDIR` that holds
//! links to the few system tools `make` needs, plus whatever fakes the test
//! lays out, and is the run's whole `PATH` and `HOME`. So neither an installed
//! Whitaker nor the Makefile's `$HOME`-relative search paths can supply a tool
//! the test did not provide.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions, OpenOptionsExt},
};

use crate::Read;

/// Links the few system tools `make lint` needs into a fresh scratch directory,
/// so a run whose `PATH` is only that directory cannot see an installed
/// Whitaker. Each test passes its own `scratch` name, since the tests run
/// concurrently and a shared directory would be cleared under one of them.
pub fn tool_directory(scratch: &str) -> Read<PathBuf> {
    let target_tmp = Dir::open_ambient_dir(env!("CARGO_TARGET_TMPDIR"), ambient_authority())?;
    target_tmp
        .remove_dir_all(scratch)
        .or_else(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        })?;
    target_tmp.create_dir(scratch)?;
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(scratch);
    for tool in ["sh", "env", "true", "make", "uname", "mkdir", "cat"] {
        let source = ["/usr/bin", "/bin"]
            .iter()
            .map(|base| Path::new(base).join(tool))
            .find(|path| path.exists())
            .ok_or_else(|| format!("no system `{tool}` to link"))?;
        // `cap_std` refuses a link to an absolute path outside its directory,
        // and a link to a system tool is exactly that.
        std::os::unix::fs::symlink(source, root.join(tool))?;
    }
    Ok(root)
}

/// Writes `files` (relative path, contents) as executable files into a fresh
/// tool directory and returns it, so a test can lay out stand-in tools.
pub fn scratch_with(scratch: &str, files: &[(&str, &str)]) -> Read<PathBuf> {
    let root = tool_directory(scratch)?;
    let dir = Dir::open_ambient_dir(&root, ambient_authority())?;
    for (path, contents) in files {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o755);
        std::io::Write::write_all(&mut dir.open_with(path, &options)?, contents.as_bytes())?;
    }
    Ok(root)
}

/// Returns a `make lint` command that runs with `root` as its whole `PATH` and
/// `HOME`, takes `cargo` as the cargo program and `whitaker` from `PATH`, and
/// is told where fakes record their runs: `WHITAKER_RECORD` and `LOG`, both in
/// `root`.
///
/// Variables the caller may have exported that change the recipe are removed,
/// and a bogus inherited `WHITAKER` is set, which the explicit `WHITAKER=`
/// argument must beat.
pub fn lint_command(root: &Path, cargo: &str) -> Command {
    let mut make = Command::new("make");
    make.args(["lint", &format!("CARGO={cargo}"), "WHITAKER=whitaker"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PATH", root)
        .env("HOME", root)
        .env("WHITAKER", "/no/such/whitaker")
        .env("WHITAKER_RECORD", root.join("record"))
        .env("LOG", root.join("log"));
    for name in [
        "RUSTFLAGS",
        "RUSTDOCFLAGS",
        "CARGO_BUILD_TARGET",
        "MAKEFLAGS",
        "MFLAGS",
        "MAKELEVEL",
    ] {
        make.env_remove(name);
    }
    make
}

/// Describes a finished run for an assertion message: its status and both
/// output streams.
pub fn diagnostics(output: &Output) -> String {
    format!(
        "ended with {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Reads a file the fakes wrote into `root`, or `None` when it does not exist.
///
/// # Errors
///
/// Any failure other than the file being absent, so an unreadable record is
/// never taken for a missing one.
pub fn recorded(root: &Path, name: &str) -> Read<Option<String>> {
    let dir = Dir::open_ambient_dir(root, ambient_authority())?;
    match dir.read_to_string(name) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
