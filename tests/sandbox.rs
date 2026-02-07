use std::process::Command;

use near_workspaces::Contract;
use sha2::{Digest, Sha256};

const EXAMPLE_PY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/example.py");
const WASM_OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/example_test.wasm");

/// Build the example contract using our CLI binary, then return the WASM bytes.
async fn deploy_example() -> Contract {
    // Build the CLI first, then use it to compile the example contract.
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("failed to build CLI");
    assert!(status.success(), "CLI build failed");

    let cli_bin = format!("{}/target/release/monty-near-cli", env!("CARGO_MANIFEST_DIR"));
    let status = Command::new(&cli_bin)
        .args(["build", EXAMPLE_PY, "-o", WASM_OUT])
        .status()
        .expect("failed to run monty-near-cli");
    assert!(status.success(), "contract build failed");

    let wasm = std::fs::read(WASM_OUT).expect("failed to read WASM");
    eprintln!("WASM size: {} bytes ({} KB)", wasm.len(), wasm.len() / 1024);

    let worker = near_workspaces::sandbox_with_version("master")
        .await
        .expect("failed to start sandbox");

    worker
        .dev_deploy(&wasm)
        .await
        .expect("failed to deploy contract")
}

fn result_string(view: &near_workspaces::result::ViewResultDetails) -> String {
    String::from_utf8(view.result.clone()).expect("non-utf8 result")
}

fn call_result_string(outcome: &near_workspaces::result::ExecutionFinalResult) -> String {
    let bytes = outcome
        .clone()
        .into_result()
        .expect("execution failed")
        .raw_bytes()
        .expect("no raw bytes");
    String::from_utf8(bytes).expect("non-utf8 result")
}

#[tokio::test]
async fn test_hello() {
    let contract = deploy_example().await;
    let result = contract.view("hello").await.expect("view failed");
    assert_eq!(result_string(&result), "Hello from Monty on NEAR!");
}

#[tokio::test]
async fn test_echo() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("echo")
        .args(b"test data 123".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "test data 123");
}

#[tokio::test]
async fn test_echo_empty() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("echo")
        .args(vec![])
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "");
}

#[tokio::test]
async fn test_greet() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("greet")
        .args(b"Alice".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "Hello, Alice!");
}

#[tokio::test]
async fn test_greet_default() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("greet")
        .args(vec![])
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "Hello, World!");
}

#[tokio::test]
async fn test_counter() {
    let contract = deploy_example().await;

    let o1 = contract
        .call("counter")
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&o1), "1");

    let o2 = contract
        .call("counter")
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&o2), "2");
}

#[tokio::test]
async fn test_get_counter() {
    let contract = deploy_example().await;

    // Increment counter
    let _ = contract
        .call("counter")
        .transact()
        .await
        .expect("call failed");

    let result = contract.view("get_counter").await.expect("view failed");
    assert_eq!(result_string(&result), "1");

    // View again — no increment
    let result2 = contract.view("get_counter").await.expect("view failed");
    assert_eq!(result_string(&result2), "1");
}

#[tokio::test]
async fn test_set_get() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("set_get")
        .args(b"hello storage".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "hello storage");
}

#[tokio::test]
async fn test_remove_key() {
    let contract = deploy_example().await;

    // First set a key
    let _ = contract
        .call("set_get")
        .args(b"temp_value".to_vec())
        .transact()
        .await
        .expect("call failed");

    // Remove it
    let outcome = contract
        .call("remove_key")
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "removed");
}

#[tokio::test]
async fn test_whoami() {
    let contract = deploy_example().await;
    let result = contract.view("whoami").await.expect("view failed");
    let s = result_string(&result);
    assert!(
        s.contains(&contract.id().to_string()),
        "expected account id in '{s}'"
    );
    assert!(s.contains("at block"), "expected 'at block' in '{s}'");
}

#[tokio::test]
async fn test_caller_info() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("caller_info")
        .transact()
        .await
        .expect("call failed");
    let s = call_result_string(&outcome);
    let account_id = contract.id().to_string();
    assert!(
        s.contains(&format!("predecessor={account_id}")),
        "missing predecessor in '{s}'"
    );
    assert!(
        s.contains(&format!("signer={account_id}")),
        "missing signer in '{s}'"
    );
    assert!(s.contains("block="), "missing block in '{s}'");
    assert!(s.contains("timestamp="), "missing timestamp in '{s}'");
}

#[tokio::test]
async fn test_hash_it() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("hash_it")
        .args(b"hello".to_vec())
        .transact()
        .await
        .expect("call failed");
    let s = call_result_string(&outcome);

    // Verify SHA-256
    let mut hasher = Sha256::new();
    hasher.update(b"hello");
    let expected_sha = hex::encode(hasher.finalize());
    assert!(
        s.contains(&format!("sha256={expected_sha}")),
        "sha256 mismatch in '{s}'"
    );

    // Verify keccak256 is present (64 hex chars)
    assert!(s.contains("keccak256="), "missing keccak256 in '{s}'");
}

#[tokio::test]
async fn test_log_and_return() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("log_and_return")
        .args(b"important event".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "logged: important event");
    assert!(
        outcome.logs().iter().any(|l| l.contains("LOG: important event")),
        "expected log message, got: {:?}",
        outcome.logs()
    );
}

#[tokio::test]
async fn test_log_and_return_default() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("log_and_return")
        .args(vec![])
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "logged: default log message");
}

#[tokio::test]
async fn test_kv_put() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("kv_put")
        .args(b"mycolor:blue".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "ok");
}

#[tokio::test]
async fn test_kv_put_error() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("kv_put")
        .args(b"no_colon_here".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "error: expected key:value");
}

#[tokio::test]
async fn test_kv_get() {
    let contract = deploy_example().await;

    // Put first
    let _ = contract
        .call("kv_put")
        .args(b"fruit:apple".to_vec())
        .transact()
        .await
        .expect("call failed");

    // Get
    let outcome = contract
        .call("kv_get")
        .args(b"fruit".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "apple");
}

#[tokio::test]
async fn test_kv_get_missing() {
    let contract = deploy_example().await;
    let outcome = contract
        .call("kv_get")
        .args(b"nonexistent_key_xyz".to_vec())
        .transact()
        .await
        .expect("call failed");
    assert_eq!(call_result_string(&outcome), "");
}

#[tokio::test]
async fn test_kv_round_trip() {
    let contract = deploy_example().await;
    let pairs = [("name", "monty"), ("version", "1.0"), ("lang", "python")];

    for (k, v) in &pairs {
        let _ = contract
            .call("kv_put")
            .args(format!("{k}:{v}").into_bytes())
            .transact()
            .await
            .expect("kv_put failed");
    }

    for (k, v) in &pairs {
        let outcome = contract
            .call("kv_get")
            .args(k.as_bytes().to_vec())
            .transact()
            .await
            .expect("kv_get failed");
        assert_eq!(call_result_string(&outcome), *v);
    }
}
