use super::*;

#[test]
fn worker_arguments_require_loopback_and_strong_token() {
    let token = "ab".repeat(32);
    let parsed = WorkerArguments::from_parts("127.0.0.1:1234", token.clone()).unwrap();
    assert_eq!(parsed.endpoint, "127.0.0.1:1234".parse().unwrap());
    assert_eq!(parsed.token, token);

    assert!(WorkerArguments::from_parts("192.0.2.1:1234", "ab".repeat(32)).is_err());
}

#[test]
fn protocol_rejects_unknown_commands() {
    assert!(serde_json::from_str::<WorkerCommand>(r#"{"command":"shell"}"#).is_err());
}
