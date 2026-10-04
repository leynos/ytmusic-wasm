//! Tests of the `make lint` recipe's Whitaker boundary and rustdoc policy.
//!
//! Each runs the real recipe with fake `cargo` and `whitaker` programs as the
//! only tools on `PATH`, over the fixtures in `fakes.rs`. Nothing here builds a
//! project: whether rustdoc really rejects a warning is rustdoc's behaviour,
//! and the recipe's part is to hand it `-D warnings` and to stop when it fails.

use std::process::Output;

use rstest::rstest;

use crate::{
    Read,
    fakes::{diagnostics, lint_command, recorded, scratch_with},
};

/// Runs `make lint` with `script` installed as the fake `whitaker` and `true`
/// as cargo, returning the run and the record the script wrote.
///
/// Cargo is replaced by `true`, so `doc` and `clippy` succeed without building.
///
/// # Errors
///
/// A script that writes no record is an error carrying the run's status and
/// both output streams, so a failure to run the fake is never read as an empty
/// record.
fn lint_with_script(scratch: &str, script: &str) -> Read<(Output, String)> {
    let root = scratch_with(scratch, &[("whitaker", script)])?;
    let output = lint_command(&root, "true").output()?;
    let record = recorded(&root, "record")?.ok_or_else(|| {
        format!(
            "the fake Whitaker left no record; `make lint` {}",
            diagnostics(&output)
        )
    })?;
    Ok((output, record))
}

/// Runs `make lint` with a fake Whitaker that records only the variables the
/// tests read (so an inherited credential never lands in a test artefact) and
/// exits with `whitaker_status`.
fn lint_with_fake_whitaker(scratch: &str, whitaker_status: i32) -> Read<(Output, String)> {
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

/// A Whitaker exit status decides `make lint`, and the fake is always run.
#[rstest]
#[case::failing("whitaker-fails", 1)]
#[case::passing("whitaker-passes", 0)]
fn lint_succeeds_exactly_when_whitaker_does(#[case] scratch: &str, #[case] status: i32) {
    let (output, record) = lint_with_fake_whitaker(scratch, status).expect("run `make lint`");
    assert_eq!(
        output.status.success(),
        status == 0,
        "`make lint` ignored Whitaker's exit status {status}: {}",
        diagnostics(&output)
    );
    assert!(!record.is_empty(), "the fake Whitaker recorded nothing");
}

#[test]
fn whitaker_builds_on_llvm_with_the_composed_flags() {
    let (output, record) = lint_with_fake_whitaker("whitaker-env", 0).expect("run `make lint`");
    let run = format!("{}; record: {record}", diagnostics(&output));
    assert!(output.status.success(), "`make lint` failed: {run}");
    let value = |name: &str| {
        record
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
    };
    assert_eq!(
        value("CARGO_UNSTABLE_CODEGEN_BACKEND").as_deref(),
        Some("true"),
        "the unstable override is wrong: {run}"
    );
    assert_eq!(
        value("CARGO_PROFILE_DEV_CODEGEN_BACKEND").as_deref(),
        Some("llvm"),
        "the backend override is wrong: {run}"
    );
    let flags = value("RUSTFLAGS");
    assert!(
        flags
            .as_deref()
            .is_some_and(|found| found.contains("-Zthreads=8")),
        "Whitaker lost the frontend flag: {flags:?}; {run}"
    );
}

/// A missing Whitaker binary skips the check with a message and `make lint`
/// still succeeds, since it is an optional tool; a present one that fails is
/// covered above. `PATH` is a scratch directory holding only the system tools
/// `make` needs, and `HOME` points at it, so neither an installed Whitaker nor
/// the Makefile's `$HOME`-relative search paths can supply one.
#[test]
fn a_missing_whitaker_skips_the_check_and_lint_succeeds() {
    let root = scratch_with("whitaker-absent", &[]).expect("prepare the tool directory");
    let output = lint_command(&root, "true")
        .output()
        .expect("run `make lint`");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "`make lint` failed without Whitaker: {}",
        diagnostics(&output)
    );
    assert!(
        stdout.contains("skipping whitaker lint"),
        "no skip message: {}",
        diagnostics(&output)
    );
    assert!(
        stdout.contains("Install whitaker"),
        "no installation guidance: {}",
        diagnostics(&output)
    );
    assert_eq!(
        recorded(&root, "record").expect("check for a record"),
        None,
        "a Whitaker run left a record although none is installed"
    );
}

/// A fake that runs but writes no record is reported with its run, not read as
/// an empty record.
#[test]
fn a_fake_that_leaves_no_record_is_reported_with_its_run() {
    let message = lint_with_script(
        "whitaker-silent",
        "#!/bin/sh\necho OUT-PAYLOAD\necho ERR-PAYLOAD >&2\nexit 3\n",
    )
    .expect_err("the fake wrote no record")
    .to_string();
    for wanted in [
        "left no record",
        "exit status: 2",
        "OUT-PAYLOAD",
        "ERR-PAYLOAD",
    ] {
        assert!(
            message.contains(wanted),
            "`{wanted}` missing from: {message}"
        );
    }
}

/// A fake `cargo` that logs each subcommand it is given, records the
/// `RUSTDOCFLAGS` of `doc` and exits with `doc_status` there and 0 otherwise.
fn fake_cargo(doc_status: i32) -> String {
    format!(
        concat!(
            "#!/bin/sh\n",
            "printf '%s\\n' \"$1\" >> \"$LOG\"\n",
            "if [ \"$1\" = doc ]; then\n",
            "  printf '%s' \"$RUSTDOCFLAGS\" > \"$LOG.doc\"\n",
            "  exit {}\n",
            "fi\n",
            "exit 0\n"
        ),
        doc_status
    )
}

/// A fake `whitaker` that logs that it ran.
const LOGGING_WHITAKER: &str = "#!/bin/sh\necho whitaker >> \"$LOG\"\nexit 0\n";

/// Runs `make lint` with the fake cargo (its `doc` exiting `doc_status`) and
/// the logging Whitaker, returning the run, the scratch root and the
/// `RUSTDOCFLAGS` given an optional inherited value.
fn lint_with_fake_cargo(
    scratch: &str,
    doc_status: i32,
    inherited: Option<&str>,
) -> Read<(Output, std::path::PathBuf)> {
    let root = scratch_with(
        scratch,
        &[
            ("cargo", &fake_cargo(doc_status)),
            ("whitaker", LOGGING_WHITAKER),
        ],
    )?;
    let mut make = lint_command(&root, &root.join("cargo").display().to_string());
    if let Some(flags) = inherited {
        make.env("RUSTDOCFLAGS", flags);
    }
    Ok((make.output()?, root))
}

/// The rustdoc flags `make lint` hands `cargo doc` are the default `-D warnings`
/// alone when the caller exports none, and the caller's value followed by the
/// default when it does, so the policy never relaxes and inherited flags survive.
#[rstest]
#[case::absent("rustdoc-absent", None, "-D warnings")]
#[case::inherited(
    "rustdoc-inherited",
    Some("--cfg docs_marker"),
    "--cfg docs_marker -D warnings"
)]
fn rustdoc_flags_deny_warnings_with_or_without_an_inherited_value(
    #[case] scratch: &str,
    #[case] inherited: Option<&str>,
    #[case] expected: &str,
) {
    let (output, root) = lint_with_fake_cargo(scratch, 0, inherited).expect("run `make lint`");
    assert!(
        output.status.success(),
        "`make lint` failed: {}",
        diagnostics(&output)
    );
    let recorded_flags = recorded(&root, "log.doc")
        .expect("read the rustdoc record")
        .unwrap_or_else(|| panic!("`cargo doc` never ran: {}", diagnostics(&output)));
    assert_eq!(recorded_flags, expected);
}

/// A failing `cargo doc` fails `make lint` and stops it before Clippy and
/// Whitaker run, so a documentation warning is never masked by a later pass.
#[test]
fn a_failing_documentation_build_stops_lint_before_clippy_and_whitaker() {
    let (output, root) = lint_with_fake_cargo("rustdoc-fails", 1, None).expect("run `make lint`");
    assert!(
        !output.status.success(),
        "`make lint` ignored a failing `cargo doc`: {}",
        diagnostics(&output)
    );
    assert_eq!(
        recorded(&root, "log").expect("read the run log").as_deref(),
        Some("doc\n"),
        "later steps ran after the failed documentation build: {}",
        diagnostics(&output)
    );
}

/// With a passing `cargo doc` the same run reaches Clippy and Whitaker in
/// order, so the stop in the failing case is the failure's doing and not a
/// recipe that never reached them.
#[test]
fn a_passing_documentation_build_lets_lint_reach_clippy_and_whitaker() {
    let (output, root) = lint_with_fake_cargo("rustdoc-passes", 0, None).expect("run `make lint`");
    assert!(
        output.status.success(),
        "`make lint` failed: {}",
        diagnostics(&output)
    );
    assert_eq!(
        recorded(&root, "log").expect("read the run log").as_deref(),
        Some("doc\nclippy\nwhitaker\n"),
        "the steps ran out of order: {}",
        diagnostics(&output)
    );
}
