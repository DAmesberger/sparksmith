//! Original JSON supplied through a stream whose metadata reports no size.
#![cfg(unix)]
use std::io::Write;
use std::process::{Command, Output, Stdio};

type Fallible = Result<(), Box<dyn std::error::Error>>;

fn stream(size: usize) -> Result<Output, Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sparksmith"))
        .args([
            "pinout",
            "preview",
            "/dev/stdin",
            "--project",
            "/no/such/project.sparksmith",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = vec![b' '; size];
    input[..2].copy_from_slice(b"{}");
    let mut stdin = child.stdin.take().ok_or("stdin")?;
    if let Err(error) = stdin.write_all(&input) {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
    drop(stdin);
    Ok(child.wait_with_output()?)
}

#[test]
fn pinout_refuses_oversized_stream_even_when_metadata_reports_zero() -> Fallible {
    let output = stream(8 * 1024 * 1024 + 1)?;
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("larger than 8 MiB"), "{error}");
    assert!(output.stdout.is_empty());
    Ok(())
}

#[test]
fn pinout_accepts_the_exact_byte_limit_before_opening_the_project() -> Fallible {
    let output = stream(8 * 1024 * 1024)?;
    let error = String::from_utf8_lossy(&output.stderr);
    // The JSON reached the next step: opening the deliberately absent project.
    assert!(error.contains("io_error"), "{error}");
    assert!(!error.contains("larger than 8 MiB"), "{error}");
    assert!(output.stdout.is_empty());
    Ok(())
}
