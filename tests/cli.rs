use assert_cmd::Command;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use predicates::prelude::*;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use wait_timeout::ChildExt;

#[test]
fn help() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rgb-multisig-hub"));
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("APP_DIRECTORY_PATH"));
}

#[test]
fn version() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rgb-multisig-hub"));
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("rgb-multisig-hub"));
}

#[test]
fn missing_config() {
    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rgb-multisig-hub"));
    cmd.arg(storage_path.to_str().unwrap())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Configuration file is missing"));
}

#[test]
fn valid_startup() {
    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path();
    let config_content = r#"
cosigner_xpubs = ["xpub1", "xpub2", "xpub3"]
threshold_colored = 2
threshold_vanilla = 2
root_public_key = "0000000000000000000000000000000000000000000000000000000000000000"
rgb_lib_version = "0.3"
"#;
    fs::write(storage_path.join("config.toml"), config_content).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rgb-multisig-hub"));
    let output = cmd
        .arg(storage_path.to_str().unwrap())
        .arg("--daemon-listening-port")
        .arg("0") // use port 0 to get a random available port
        .timeout(std::time::Duration::from_secs(2))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Listening on"));
    assert!(storage_path.join("logs").exists());
}

#[test]
#[cfg(unix)]
fn sigterm_shutdown() {
    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path();
    let config_content = r#"
cosigner_xpubs = ["xpub1", "xpub2", "xpub3"]
threshold_colored = 2
threshold_vanilla = 2
root_public_key = "0000000000000000000000000000000000000000000000000000000000000000"
rgb_lib_version = "0.3"
"#;
    fs::write(storage_path.join("config.toml"), config_content).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rgb-multisig-hub"))
        .arg(storage_path.to_str().unwrap())
        .arg("--daemon-listening-port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut started = false;
    for line in reader.lines().take(100) {
        if let Ok(line) = line
            && line.contains("Listening on")
        {
            started = true;
            break;
        }
    }
    assert!(started);
    std::thread::sleep(Duration::from_millis(100));
    let pid = Pid::from_raw(child.id() as i32);
    signal::kill(pid, Signal::SIGTERM).unwrap();
    let result = child.wait_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.is_some());
    let exit_status = result.unwrap();
    assert!(exit_status.success() && exit_status.code() == Some(0));
}

#[test]
#[cfg(unix)]
fn sigint_shutdown() {
    let temp_dir = TempDir::new().unwrap();
    let storage_path = temp_dir.path();
    let config_content = r#"
cosigner_xpubs = ["xpub1", "xpub2", "xpub3"]
threshold_colored = 2
threshold_vanilla = 2
root_public_key = "0000000000000000000000000000000000000000000000000000000000000000"
rgb_lib_version = "0.3"
"#;
    fs::write(storage_path.join("config.toml"), config_content).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_rgb-multisig-hub"))
        .arg(storage_path.to_str().unwrap())
        .arg("--daemon-listening-port")
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);
    let mut started = false;
    for line in reader.lines().take(100) {
        if let Ok(line) = line
            && line.contains("Listening on")
        {
            started = true;
            break;
        }
    }
    assert!(started);
    std::thread::sleep(Duration::from_millis(100));
    let pid = Pid::from_raw(child.id() as i32);
    signal::kill(pid, Signal::SIGINT).unwrap();
    let result = child.wait_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.is_some());
    let exit_status = result.unwrap();
    assert!(exit_status.success() && exit_status.code() == Some(0));
}
