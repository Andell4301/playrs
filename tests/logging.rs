use playrs::GooglePlayApi;
use std::process::{Command, Output};

fn run_auth_info(rust_log: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playrs"))
        .env("RUST_LOG", rust_log)
        .args(["--gsf-id", "test-gsf-id", "--auth-token", "test-auth-token", "auth-info"])
        .output()
        .expect("failed to run playrs")
}

#[test]
fn rust_log_can_suppress_startup_logs() {
    for filter in ["off", "playrs=error"] {
        let output = run_auth_info(filter);

        assert!(output.status.success(), "playrs failed with RUST_LOG={filter}");
        assert!(output.stderr.is_empty(), "playrs wrote logs with RUST_LOG={filter}");

        let stdout = String::from_utf8(output.stdout).expect("playrs wrote non-UTF-8 output");
        assert!(!stdout.is_empty(), "auth-info should still write command output to stdout");
        assert!(
            stdout.lines().all(|line| line.starts_with("GSF ID:") || line.starts_with("Auth Token:")),
            "playrs mixed logs into stdout with RUST_LOG={filter}"
        );
    }
}

#[test]
fn logs_use_stderr() {
    let output = run_auth_info("playrs=info");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("playrs wrote non-UTF-8 output");
    let stderr = String::from_utf8(output.stderr).expect("playrs wrote non-UTF-8 logs");

    assert!(stdout.lines().all(|line| line.starts_with("GSF ID:") || line.starts_with("Auth Token:")));
    assert!(stderr.contains("auth ready"));
    assert!(!stderr.contains("test-auth-token"));
}

#[test]
fn auth_info_prints_credentials() {
    let output = run_auth_info("off");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GSF ID: test-gsf-id"));
    assert!(stdout.contains("Auth Token: test-auth-token"));
}

#[test]
fn auth_data_debug_output_previews_tokens_and_redacts_other_credentials() {
    let cases = [
        (
            GooglePlayApi::new_from_gsf_id("debug-gsf-id", "debug-auth-token", None::<String>, None)
                .expect("failed to create API from GSF credentials"),
            "debug-gsf-id",
            "debug-auth-token",
        ),
        (
            GooglePlayApi::new_from_aas_token("debug-aas-token", "debug-aas-email@example.invalid", None::<String>, None)
                .expect("failed to create API from AAS credentials"),
            "debug-aas-email@example.invalid",
            "debug-aas-token",
        ),
        (
            GooglePlayApi::new_from_oauth_login_token("debug-oauth-token", "debug-oauth-email@example.invalid", None::<String>, None)
                .expect("failed to create API from OAuth credentials"),
            "debug-oauth-email@example.invalid",
            "debug-oauth-token",
        ),
    ];

    for (api, hidden_credential, token) in cases {
        let rendered = format!("{:?}", api.get_auth_data());
        assert!(!rendered.contains(hidden_credential), "AuthData debug output exposed a credential");
        assert!(!rendered.contains(token), "AuthData debug output exposed a complete token");
        assert!(rendered.contains("deb…ken"), "AuthData debug output did not include a token preview");
    }
}

#[test]
fn span_fields_escape_newlines() {
    let output = Command::new(env!("CARGO_BIN_EXE_playrs"))
        .env("RUST_LOG", "playrs=info")
        .args([
            "--gsf-id",
            "test-gsf-id",
            "--auth-token",
            "test-auth-token",
            "download",
            "--package-name",
            "evil\nINJECT",
            "--output-dir",
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--version-code",
            "1",
            "--offer-type",
            "1",
        ])
        .output()
        .expect("failed to run playrs");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("playrs wrote non-UTF-8 logs");
    assert!(!stderr.contains("evil\nINJECT"));
    assert!(stderr.contains(r#"package_name="evil\nINJECT""#));
}

#[test]
fn command_errors_do_not_repeat_the_source() {
    let output = Command::new(env!("CARGO_BIN_EXE_playrs"))
        .env("RUST_LOG", "off")
        .args(["--gsf-id", "test-gsf-id", "--auth-token", "test-auth-token", "--device-name", "missing-test-device", "auth-info"])
        .output()
        .expect("failed to run playrs");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("playrs wrote a non-UTF-8 error");
    assert_eq!(stderr.matches("Device with codename").count(), 1);
}
