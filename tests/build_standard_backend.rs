//! Contract tests for the build standard's code-generation backend.
//!
//! The standard makes Cranelift the development profile's backend, which needs
//! the unstable Cargo key and the toolchain component. Two builds must not use
//! it: coverage, because Cranelift cannot build `-Cinstrument-coverage`, and
//! Whitaker's Dylint driver, which builds outside the workspace configuration.
//! These tests hold the configuration, the coverage step's overrides and the
//! `make lint` Whitaker boundary, which the flag contract does not reach.
//!
//! File access goes through `cap_std` directory handles: one rooted at the
//! crate manifest directory, and one at a scratch directory under
//! `CARGO_TARGET_TMPDIR` for the fake tools.

use std::{
    error::Error,
    process::{Command, Output},
};

use cap_std::{ambient_authority, fs::Dir};
use rstest::rstest;

/// The result of a reader, which the tests unwrap.
type Read<T> = Result<T, Box<dyn Error>>;

/// The toolchain component Cranelift needs.
const CRANELIFT_COMPONENT: &str = "rustc-codegen-cranelift-preview";

/// The overrides a build outside the workspace configuration needs to take
/// LLVM: the unstable feature, then the profile's backend.
const LLVM_OVERRIDES: [&str; 2] = [
    "CARGO_UNSTABLE_CODEGEN_BACKEND",
    "CARGO_PROFILE_DEV_CODEGEN_BACKEND",
];

/// Reads a file relative to the crate manifest directory.
fn read(path: &str) -> Read<String> {
    let root = Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), ambient_authority())?;
    Ok(root.read_to_string(path)?)
}

/// Reads a TOML file relative to the crate manifest directory.
fn read_toml(path: &str) -> Read<toml::Value> {
    Ok(toml::from_str(&read(path)?)?)
}

/// Follows a path of table keys through a TOML value.
fn value_at<'a>(root: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    path.iter().try_fold(root, |value, key| value.get(key))
}

#[test]
fn cranelift_is_the_development_backend() {
    let config = read_toml(".cargo/config.toml").expect("read the Cargo configuration");
    assert_eq!(
        value_at(&config, &["unstable", "codegen-backend"]).and_then(toml::Value::as_bool),
        Some(true),
        "`[unstable] codegen-backend = true` is what lets Cargo accept the profile key"
    );
    assert_eq!(
        value_at(&config, &["profile", "dev", "codegen-backend"]).and_then(toml::Value::as_str),
        Some("cranelift"),
        "the development profile no longer selects Cranelift"
    );
    let toolchain = read_toml("rust-toolchain.toml").expect("read the toolchain file");
    let has_component = value_at(&toolchain, &["toolchain", "components"])
        .and_then(toml::Value::as_array)
        .is_some_and(|components| {
            components
                .iter()
                .any(|c| c.as_str() == Some(CRANELIFT_COMPONENT))
        });
    assert!(
        has_component,
        "the pinned toolchain lacks `{CRANELIFT_COMPONENT}`"
    );
}

/// Returns the number of leading spaces on a line.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Returns whether a line begins a YAML sequence item, which is a workflow step.
fn starts_step(line: &str) -> bool {
    line.trim_start().starts_with("- ")
}

/// Returns the lines of the workflow step that runs the coverage action.
///
/// A step starts at a `- ` line and runs to the next `- ` line at the same or
/// a shallower indent, so nested lists inside the step stay with it.
fn coverage_step(workflow: &str) -> Vec<&str> {
    let lines: Vec<&str> = workflow.lines().collect();
    let step_starts: Vec<usize> = (0..lines.len())
        .filter(|&i| lines.get(i).is_some_and(|line| starts_step(line)))
        .collect();
    let bounds = lines
        .iter()
        .position(|line| line.contains("generate-coverage@"))
        .and_then(|uses| {
            let start = step_starts.iter().copied().rfind(|&i| i <= uses)?;
            let start_indent = indent(lines.get(start)?);
            let end = step_starts
                .iter()
                .copied()
                .find(|&i| {
                    i > uses
                        && lines
                            .get(i)
                            .is_some_and(|line| indent(line) <= start_indent)
                })
                .unwrap_or(lines.len());
            Some(start..end)
        });
    bounds
        .and_then(|range| lines.get(range))
        .map(<[&str]>::to_vec)
        .unwrap_or_default()
}

#[test]
fn coverage_builds_on_llvm() {
    let workflow = read(".github/workflows/ci.yml").expect("read ci.yml");
    let step = coverage_step(&workflow);
    assert!(!step.is_empty(), "ci.yml has no `generate-coverage` step");
    for name in LLVM_OVERRIDES {
        let wanted = if name == "CARGO_UNSTABLE_CODEGEN_BACKEND" {
            "true"
        } else {
            "llvm"
        };
        let has_override = step.iter().any(|line| {
            let mut words = line.trim().splitn(2, ':');
            words.next() == Some(name)
                && words
                    .next()
                    .is_some_and(|value| value.trim().trim_matches('"') == wanted)
        });
        assert!(
            has_override,
            "the coverage step does not set {name}={wanted}"
        );
    }
}

/// Runs `make lint` with fake tools first on `PATH`, returning whether it
/// succeeded and the environment the fake Whitaker recorded.
///
/// Cargo is replaced by `true`, so `doc` and `clippy` succeed without
/// building. The fake Whitaker writes its environment to a file and exits with
/// `whitaker_status`. Each test passes its own `scratch` directory name, since
/// the tests run concurrently and a shared directory would be cleared under one
/// of them.
#[cfg(unix)]
fn lint_with_fake_whitaker(scratch: &str, whitaker_status: i32) -> Read<(Output, String)> {
    // Record only the variables the tests read, so an inherited credential
    // never lands in a test artefact.
    let script = format!(
        concat!(
            "#!/bin/sh\n",
            "for name in RUSTFLAGS CARGO_UNSTABLE_CODEGEN_BACKEND CARGO_PROFILE_DEV_CODEGEN_BACKEND; do\n",
            "  eval \"value=\\${{$name-__unset__}}\"\n",
            "  [ \"$value\" = __unset__ ] || echo \"$name=$value\"\n",
            "done > \"$WHITAKER_RECORD\"\n",
            "exit {}\n"
        ),
        whitaker_status
    );
    lint_with_script(scratch, &script)
}

/// Runs `make lint` with `script` installed as the fake `whitaker`, returning
/// the run and the record the script wrote.
///
/// # Errors
///
/// A script that writes no record is an error carrying the run's status and
/// both output streams, so a failure to run the fake is never read as an empty
/// record.
#[cfg(unix)]
fn lint_with_script(scratch: &str, script: &str) -> Read<(Output, String)> {
    use cap_std::fs::{OpenOptions, OpenOptionsExt};

    // The tool directory is rebuilt clean, so a record from an earlier run
    // cannot survive, and it holds the only tools on `PATH`.
    let root = tool_directory(scratch)?;
    let dir = Dir::open_ambient_dir(&root, ambient_authority())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o755);
    std::io::Write::write_all(&mut dir.open_with("whitaker", &options)?, script.as_bytes())?;
    let output = Command::new("make")
        // `WHITAKER=whitaker` beats an inherited `WHITAKER`, which the bogus
        // value below would otherwise make the recipe run.
        .args(["lint", "CARGO=true", "WHITAKER=whitaker"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PATH", &root)
        .env("WHITAKER", "/no/such/whitaker")
        .env("WHITAKER_RECORD", root.join("record"))
        // The Makefile searches `$HOME`-relative tool directories too, so point
        // it at the scratch directory rather than a real home.
        .env("HOME", &root)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env_remove("MAKELEVEL")
        .output()?;
    // A missing record means the fake never ran; surface that rather than
    // reading it as an empty successful one.
    let record = dir.read_to_string("record").map_err(|error| {
        format!(
            "the fake Whitaker left no record ({error}); `make lint` exited {} with stdout {} and stderr {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    Ok((output, record))
}

/// A Whitaker exit status decides `make lint`, and the fake is always run.
#[cfg(unix)]
#[rstest]
#[case::failing("whitaker-fails", 1)]
#[case::passing("whitaker-passes", 0)]
fn lint_succeeds_exactly_when_whitaker_does(#[case] scratch: &str, #[case] status: i32) {
    let (output, record) = lint_with_fake_whitaker(scratch, status).expect("run `make lint`");
    assert_eq!(
        output.status.success(),
        status == 0,
        "`make lint` ignored Whitaker's exit status {status}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!record.is_empty(), "the fake Whitaker recorded nothing");
}

#[cfg(unix)]
#[test]
fn whitaker_builds_on_llvm_with_the_composed_flags() {
    let (output, record) = lint_with_fake_whitaker("whitaker-env", 0).expect("run `make lint`");
    assert!(
        output.status.success(),
        "`make lint` failed ({}): {}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value = |name: &str| {
        record
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
    };
    assert_eq!(
        value("CARGO_UNSTABLE_CODEGEN_BACKEND").as_deref(),
        Some("true")
    );
    assert_eq!(
        value("CARGO_PROFILE_DEV_CODEGEN_BACKEND").as_deref(),
        Some("llvm")
    );
    let flags = value("RUSTFLAGS").expect("Whitaker received no RUSTFLAGS");
    assert!(
        flags.contains("-Zthreads=8"),
        "Whitaker lost the frontend flag: {flags}"
    );
}

/// Links the few system tools `make lint` needs into a scratch directory, so a
/// run whose `PATH` is only that directory cannot see an installed Whitaker.
#[cfg(unix)]
fn tool_directory(scratch: &str) -> Read<std::path::PathBuf> {
    let target_tmp = Dir::open_ambient_dir(env!("CARGO_TARGET_TMPDIR"), ambient_authority())?;
    target_tmp
        .remove_dir_all(scratch)
        .or_else(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        })?;
    target_tmp.create_dir(scratch)?;
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(scratch);
    for tool in ["sh", "env", "true", "make", "uname"] {
        let source = ["/usr/bin", "/bin"]
            .iter()
            .map(|base| std::path::Path::new(base).join(tool))
            .find(|path| path.exists())
            .ok_or_else(|| format!("no system `{tool}` to link"))?;
        // `cap_std` refuses a link to an absolute path outside its directory,
        // and a link to a system tool is exactly that.
        std::os::unix::fs::symlink(source, root.join(tool))?;
    }
    Ok(root)
}

/// A missing Whitaker binary skips the check with a message and `make lint`
/// still succeeds, since it is an optional tool; a present one that fails is
/// covered above. `PATH` is a scratch directory holding only the system tools
/// `make` needs, and `HOME` points at it, so neither an installed Whitaker nor
/// the Makefile's `$HOME`-relative search paths can supply one.
#[cfg(unix)]
#[test]
fn a_missing_whitaker_skips_the_check_and_lint_succeeds() {
    let tools = tool_directory("whitaker-absent").expect("prepare the tool directory");
    let output = Command::new("make")
        .args(["lint", "CARGO=true", "WHITAKER=whitaker"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("WHITAKER", "/no/such/whitaker")
        .env("PATH", &tools)
        .env("HOME", &tools)
        .env("WHITAKER_RECORD", tools.join("record"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env_remove("MAKELEVEL")
        .output()
        .expect("run `make lint`");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "`make lint` failed without Whitaker ({}): {stdout}{stderr}",
        output.status
    );
    assert!(
        stdout.contains("skipping whitaker lint"),
        "no skip message in: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("Install whitaker"),
        "no installation guidance in: {stdout}{stderr}"
    );
    assert!(
        !tools.join("record").exists(),
        "a Whitaker run left a record although none is installed"
    );
}

/// A fake that runs but writes no record is reported with its run, not read as
/// an empty record.
#[cfg(unix)]
#[test]
fn a_fake_that_leaves_no_record_is_reported_with_its_run() {
    let message = lint_with_script("whitaker-silent", "#!/bin/sh\nexit 0\n")
        .expect_err("the fake wrote no record")
        .to_string();
    for wanted in ["left no record", "exited", "stdout", "stderr"] {
        assert!(
            message.contains(wanted),
            "`{wanted}` missing from: {message}"
        );
    }
}
