use std::process::Command;

#[test]
fn invalid_arguments_return_usage_and_exit_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_lyra-upgrade"))
        .arg("apply")
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "usage: lyra-upgrade [inspect]\n"
    );
}
