#[test]
fn test_cli_version_flag() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("gdown");
    cmd.arg("--version").assert().success();
}

#[test]
fn test_cli_help_flag() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("gdown");
    cmd.arg("--help").assert().success();
}

#[test]
fn test_cli_no_args_fails() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("gdown");
    cmd.assert().failure();
}
