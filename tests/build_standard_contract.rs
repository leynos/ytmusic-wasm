//! Contract tests for the Rust build standard.
//!
//! The standard makes the parallel `rustc` frontend the default for every
//! development build and mold the default linker on Linux. Cargo reads both
//! from `.cargo/config.toml`, but it applies a single `rustflags` source rather
//! than merging them, and an assigned `RUSTFLAGS` replaces every source. So the
//! flags must be repeated in each configuration source, restated wherever the
//! Makefile assigns `RUSTFLAGS` for a development target, and kept out of the
//! coverage and release recipes, which measure or ship and so stay on the
//! default flags.
//!
//! The Makefile clauses run `make -n` and read the commands it would run,
//! rather than the Makefile's text, so a flag lost through a variable or a
//! recipe edit fails here. Each assigned value is expanded by the shell, with
//! and without an inherited `RUSTFLAGS`, exactly as the recipe would expand
//! it. The clauses run once as a Linux host and once as a macOS host, because
//! mold is added on Linux alone. File access goes through a
//! `cap_std` directory handle rooted at the crate manifest directory.

use std::{error::Error, process::Command};

use cap_std::{ambient_authority, fs::Dir};

/// The parallel-frontend flag every `rustflags` source must carry.
const THREADS_FLAG: &str = "-Zthreads=8";

/// The linker flag the Linux source must add, normalized to one token.
const MOLD_FLAG: &str = "-Clink-arg=-fuse-ld=mold";

/// Target table keys that apply on Linux alone.
const LINUX_TABLES: [&str; 2] = ["x86_64-unknown-linux-gnu", "cfg(target_os = \"linux\")"];

/// Makefile targets that build for development. A command in one either
/// assigns `RUSTFLAGS` with the standard flags or assigns none and so takes
/// the configuration's.
const DEVELOPMENT_TARGETS: [&str; 3] = ["test", "lint", "build"];

/// Makefile targets that measure or ship, so every command assigns
/// `RUSTFLAGS` and none carries a standard flag.
const HELD_OUT_TARGETS: [&str; 1] = ["release"];

/// Development targets that must assign `RUSTFLAGS` in at least one command,
/// so the restatement checks above cannot pass by finding nothing to check.
const ASSIGNING_TARGETS: [&str; 2] = ["test", "build"];

/// A caller's own flags, to prove a recipe keeps the standard when it
/// inherits an exported `RUSTFLAGS`.
const INHERITED: &str = "-D warnings";

/// The result of a reader, which the tests unwrap.
type Read<T> = Result<T, Box<dyn Error>>;

/// Joins `-C value` pairs into `-Cvalue`, so both spellings compare equal.
fn normalized(flags: &[String]) -> Vec<String> {
    let mut joined: Vec<String> = Vec::new();
    for flag in flags {
        match joined.last_mut() {
            Some(last) if last == "-C" => *last = format!("-C{flag}"),
            _ => joined.push(flag.clone()),
        }
    }
    joined
}

/// Returns whether a flag list names one flag.
fn names(flags: &[String], flag: &str) -> bool {
    flags.iter().any(|candidate| candidate == flag)
}

/// Reads one table's `rustflags` as a list of strings, if it has one.
fn table_flags(table: &toml::Value) -> Option<Vec<String>> {
    let flags = table.get("rustflags")?.as_array()?;
    Some(
        flags
            .iter()
            .filter_map(|flag| flag.as_str().map(str::to_owned))
            .collect(),
    )
}

/// Returns every `rustflags` source in the configuration, by table name.
fn sources() -> Read<Vec<(String, Vec<String>)>> {
    let root = Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), ambient_authority())?;
    let text = root.read_to_string(".cargo/config.toml")?;
    let config: toml::Value = toml::from_str(&text)?;
    let mut found = Vec::new();
    if let Some(flags) = config.get("build").and_then(table_flags) {
        found.push(("build".to_owned(), normalized(&flags)));
    }
    if let Some(targets) = config.get("target").and_then(toml::Value::as_table) {
        for (key, table) in targets {
            if let Some(flags) = table_flags(table) {
                found.push((key.clone(), normalized(&flags)));
            }
        }
    }
    Ok(found)
}

/// Expands an assigned value as the recipe's shell would, with or without an
/// inherited `RUSTFLAGS`, and splits it into normalized flags.
fn expanded(value: &str, inherited: Option<&str>) -> Read<Vec<String>> {
    let mut shell = Command::new("bash");
    shell
        .args(["-c", &format!("printf '%s' \"{value}\"")])
        .env_remove("RUSTFLAGS");
    if let Some(flags) = inherited {
        shell.env("RUSTFLAGS", flags);
    }
    let output = shell.output()?;
    if !output.status.success() {
        return Err(format!("the shell could not expand `{value}`").into());
    }
    let words: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    Ok(normalized(&words))
}

/// Returns, for each cargo or whitaker command `make -n TARGET` would run on
/// the named host, the `RUSTFLAGS` it assigns (expanded under `inherited`),
/// or `None` when it assigns none.
fn make_rustflags(
    target: &str,
    host: &str,
    inherited: Option<&str>,
) -> Read<Vec<Option<Vec<String>>>> {
    let output = Command::new("make")
        .args(["-n", "-B", &format!("BUILD_HOST_OS={host}"), target])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()?;
    if !output.status.success() {
        return Err(format!("`make -n {target}` failed").into());
    }
    // A recipe continued with a trailing backslash is one command.
    let stdout = String::from_utf8_lossy(&output.stdout).replace("\\\n", " ");
    let mut commands: Vec<Option<Vec<String>>> = Vec::new();
    for line in stdout
        .lines()
        .filter(|line| line.contains("cargo") || line.contains("whitaker"))
    {
        let assigned = match line.split_once("RUSTFLAGS=\"") {
            Some((_, rest)) => {
                let (value, _) = rest
                    .split_once('"')
                    .ok_or_else(|| format!("unterminated RUSTFLAGS in `{line}`"))?;
                Some(expanded(value, inherited)?)
            }
            // Any other spelling still replaces the configuration's sources,
            // so a form this reader cannot parse fails rather than passing.
            None if line.contains("RUSTFLAGS=") => {
                return Err(format!("unreadable RUSTFLAGS assignment in `{line}`").into());
            }
            None => None,
        };
        commands.push(assigned);
    }
    if commands.is_empty() {
        return Err(format!("`make -n {target}` runs no cargo command").into());
    }
    Ok(commands)
}

/// Checks every development target on one host: an assigned `RUSTFLAGS`
/// carries the frontend flag, and carries mold exactly when the host is Linux.
fn check_development_targets(
    host: &str,
    expects_mold: bool,
    inherited: Option<&str>,
) -> Read<Vec<String>> {
    let mut problems = Vec::new();
    for target in DEVELOPMENT_TARGETS {
        // An empty assignment is `Some(vec![])` and is checked like any other.
        for flags in make_rustflags(target, host, inherited)?
            .into_iter()
            .flatten()
        {
            if !names(&flags, THREADS_FLAG) {
                problems.push(format!(
                    "`make {target}` on {host} drops {THREADS_FLAG}: {flags:?}"
                ));
            }
            if names(&flags, MOLD_FLAG) != expects_mold {
                problems.push(format!(
                    "`make {target}` on {host} gets mold wrong: {flags:?}"
                ));
            }
        }
    }
    Ok(problems)
}

#[test]
fn every_rustflags_source_carries_the_parallel_frontend() {
    let found = sources().expect("read the configuration sources");
    assert!(
        found.iter().any(|(key, _)| key == "build"),
        "no [build] rustflags for non-Linux hosts"
    );
    let missing: Vec<&str> = found
        .iter()
        .filter(|(_, flags)| !names(flags, THREADS_FLAG))
        .map(|(key, _)| key.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "{THREADS_FLAG} missing from {missing:?}"
    );
}

#[test]
fn mold_is_confined_to_linux() {
    let found = sources().expect("read the configuration sources");
    let linux: Vec<_> = found
        .iter()
        .filter(|(key, _)| LINUX_TABLES.contains(&key.as_str()))
        .collect();
    assert!(!linux.is_empty(), "no Linux target table carries rustflags");
    assert!(
        linux.iter().all(|(_, flags)| names(flags, MOLD_FLAG)),
        "a Linux table lost mold"
    );
    let wider: Vec<&str> = found
        .iter()
        .filter(|(key, flags)| !LINUX_TABLES.contains(&key.as_str()) && names(flags, MOLD_FLAG))
        .map(|(key, _)| key.as_str())
        .collect();
    assert!(wider.is_empty(), "mold named beyond Linux in {wider:?}");
}

#[test]
fn sources_differ_only_by_the_linker() {
    let mut stripped: Vec<Vec<String>> = sources()
        .expect("read the configuration sources")
        .into_iter()
        .map(|(_, flags)| flags.into_iter().filter(|flag| flag != MOLD_FLAG).collect())
        .collect();
    stripped.dedup();
    assert_eq!(
        stripped.len(),
        1,
        "rustflags sources disagree: {stripped:?}"
    );
}

#[test]
fn development_targets_restate_both_flags_on_linux() {
    let problems = check_development_targets("Linux", true, None).expect("read `make -n` output");
    assert!(problems.is_empty(), "{problems:#?}");
    for target in ASSIGNING_TARGETS {
        let assigned = make_rustflags(target, "Linux", None)
            .expect("read `make -n` output")
            .into_iter()
            .flatten()
            .count();
        assert!(assigned > 0, "`make {target}` assigns no RUSTFLAGS");
    }
}

#[test]
fn development_targets_keep_the_standard_under_inherited_rustflags() {
    let problems =
        check_development_targets("Linux", true, Some(INHERITED)).expect("read `make -n` output");
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn development_targets_keep_the_frontend_but_not_mold_elsewhere() {
    let problems = check_development_targets("Darwin", false, None).expect("read `make -n` output");
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Coverage measures and release ships, so both stay on the default flags.
/// Every command must assign `RUSTFLAGS`, since only an assignment displaces
/// the configuration's sources.
#[test]
fn coverage_and_release_take_neither_flag() {
    for target in HELD_OUT_TARGETS {
        for assigned in make_rustflags(target, "Linux", None).expect("read `make -n` output") {
            let flags = assigned.unwrap_or_else(|| {
                panic!("`make {target}` runs a command that takes the configuration's flags")
            });
            assert!(
                !names(&flags, THREADS_FLAG),
                "`make {target}` takes {THREADS_FLAG}"
            );
            assert!(
                !names(&flags, MOLD_FLAG),
                "`make {target}` takes {MOLD_FLAG}"
            );
        }
    }
}
