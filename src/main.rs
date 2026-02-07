use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use monty::MontyRun;
use ruff_python_ast::Stmt;
use ruff_python_parser::parse_module;

// ---------------------------------------------------------------------------
// Template files — embedded at compile time from template/
// ---------------------------------------------------------------------------

const TEMPLATE_CARGO_TOML: &str = include_str!("../template/Cargo.toml");
const TEMPLATE_RUST_TOOLCHAIN: &str = include_str!("../template/rust-toolchain.toml");
const TEMPLATE_CARGO_CONFIG: &str = include_str!("../template/.cargo/config.toml");
const TEMPLATE_LIB_RS: &str = include_str!("../template/src/lib.rs");

// Markers in template/src/lib.rs where generated code is spliced in.
const MARKER_BYTECODE: &str = "// @MONTY_BYTECODE_STATICS";
const MARKER_EXPORTS: &str = "// @MONTY_EXPORTS";

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "monty-near",
    about = "Compile Python to NEAR WASM smart contracts"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a Python file into a NEAR-deployable WASM contract
    Build {
        /// Path to the Python source file
        input: PathBuf,

        /// Output path for the WASM binary
        #[arg(short, long, default_value = "contract.wasm")]
        output: PathBuf,

        /// Build for compatibility with the current production NearVM (Wasmer).
        ///
        /// Uses nightly Rust with -Zbuild-std and -Ctarget-cpu=mvp to avoid
        /// emitting bulk-memory WASM instructions that NearVM rejects. Without
        /// this flag, the output targets the upcoming Wasmtime-based runtime
        /// (nearcore 2.12+) which supports bulk-memory natively.
        #[arg(long)]
        compat: bool,

        /// Skip wasm-opt post-processing.
        ///
        /// By default the build runs `wasm-opt -Oz` on the output to reduce
        /// WASM size. Pass this flag to skip that step (e.g. for faster
        /// iteration or if wasm-opt is not installed).
        #[arg(long)]
        no_wasm_opt: bool,
    },
}

// ---------------------------------------------------------------------------
// External NEAR functions available to Python contracts
// ---------------------------------------------------------------------------

fn near_external_functions() -> Vec<String> {
    [
        // Existing
        "value_return",
        "input",
        "log",
        "storage_write",
        "storage_read",
        "storage_remove",
        "storage_has_key",
        "current_account_id",
        "predecessor_account_id",
        "signer_account_id",
        "block_height",
        "block_timestamp",
        "sha256",
        "keccak256",
        // Context
        "signer_account_pk",
        "epoch_height",
        "storage_usage",
        // Economics
        "account_balance",
        "account_locked_balance",
        "attached_deposit",
        "prepaid_gas",
        "used_gas",
        // Math
        "random_seed",
        "keccak512",
        "ripemd160",
        "ecrecover",
        "ed25519_verify",
        // Promises
        "promise_create",
        "promise_then",
        "promise_and",
        "promise_batch_create",
        "promise_batch_then",
        "promise_results_count",
        "promise_result",
        "promise_return",
        // Promise batch actions
        "promise_batch_action_create_account",
        "promise_batch_action_deploy_contract",
        "promise_batch_action_function_call",
        "promise_batch_action_function_call_weight",
        "promise_batch_action_transfer",
        "promise_batch_action_stake",
        "promise_batch_action_add_key_with_full_access",
        "promise_batch_action_add_key_with_function_call",
        "promise_batch_action_delete_key",
        "promise_batch_action_delete_account",
        // Validator
        "validator_stake",
        "validator_total_stake",
        // Alt BN128
        "alt_bn128_g1_multiexp",
        "alt_bn128_g1_sum",
        "alt_bn128_pairing_check",
        // BLS12-381
        "bls12381_p1_sum",
        "bls12381_p2_sum",
        "bls12381_g1_multiexp",
        "bls12381_g2_multiexp",
        "bls12381_map_fp_to_g1",
        "bls12381_map_fp2_to_g2",
        "bls12381_pairing_check",
        "bls12381_p1_decompress",
        "bls12381_p2_decompress",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------------------------------------------------------------------
// Python source parsing — find exported top-level functions
// ---------------------------------------------------------------------------

/// Find top-level function names that don't start with `_`.
///
/// Uses ruff's Python parser (the same parser Monty uses) to walk the AST
/// rather than fragile string matching on `def ` prefixes.
fn find_exported_functions(source: &str) -> Result<Vec<String>> {
    let parsed = parse_module(source).map_err(|e| anyhow::anyhow!("Python parse error: {e}"))?;
    let module = parsed.into_syntax();

    let mut functions = Vec::new();
    for stmt in &module.body {
        if let Stmt::FunctionDef(func) = stmt {
            let name = func.name.as_str();
            if !name.starts_with('_') {
                functions.push(name.to_string());
            }
        }
    }
    Ok(functions)
}

// ---------------------------------------------------------------------------
// Pre-compilation — compile source + dispatcher to single Monty bytecode blob
// ---------------------------------------------------------------------------

/// Generate a Python dispatcher that routes `_method` to the correct function.
fn generate_dispatcher(method_names: &[String]) -> String {
    let mut dispatcher = String::new();
    for (i, name) in method_names.iter().enumerate() {
        if i == 0 {
            dispatcher.push_str(&format!("if _method == \"{name}\":\n    {name}()\n"));
        } else {
            dispatcher.push_str(&format!("elif _method == \"{name}\":\n    {name}()\n"));
        }
    }
    dispatcher
}

/// Compile the full source with a dispatcher into a single bytecode blob.
fn precompile_contract(source: &str, method_names: &[String]) -> Result<Vec<u8>> {
    let dispatcher = generate_dispatcher(method_names);
    let program = format!("{source}\n\n{dispatcher}");
    let external_functions = near_external_functions();

    // `_method` is an input variable — the Rust runtime passes the method name at call time.
    let runner = MontyRun::new(
        program,
        "contract.py",
        vec!["_method".to_string()],
        external_functions,
    )
    .map_err(|e| anyhow::anyhow!("compilation failed: {e}"))?;

    runner.dump().context("serialization failed")
}

// ---------------------------------------------------------------------------
// Code generation — splice generated code into the template
// ---------------------------------------------------------------------------

/// Generate the `lib.rs` source with a single shared bytecode blob and
/// thin `#[no_mangle]` exports that pass the method name.
fn generate_lib_rs(method_names: &[String]) -> String {
    let bytecode_static = "static CONTRACT_BYTECODE: &[u8] = include_bytes!(\"contract.bin\");\n";

    let mut exports = String::new();
    for name in method_names {
        exports.push_str(&format!(
            "#[no_mangle]\npub extern \"C\" fn {name}() {{\n    run_method(CONTRACT_BYTECODE, \"{name}\");\n}}\n\n",
        ));
    }

    TEMPLATE_LIB_RS
        .replace(MARKER_BYTECODE, bytecode_static)
        .replace(MARKER_EXPORTS, &exports)
}

// ---------------------------------------------------------------------------
// Project scaffolding — write the temporary Rust project to disk
// ---------------------------------------------------------------------------

fn write_project(dir: &Path, method_names: &[String], bytecode: &[u8], compat: bool) -> Result<()> {
    fs::write(dir.join("Cargo.toml"), TEMPLATE_CARGO_TOML)?;

    if compat {
        // Nightly toolchain with rust-src for -Zbuild-std
        fs::write(
            dir.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"nightly\"\ntargets = [\"wasm32-unknown-unknown\"]\ncomponents = [\"rust-src\"]\n",
        )?;

        // Add -Ctarget-cpu=mvp to disable bulk-memory instructions
        let cargo_dir = dir.join(".cargo");
        fs::create_dir_all(&cargo_dir)?;
        fs::write(
            cargo_dir.join("config.toml"),
            "[build]\ntarget = \"wasm32-unknown-unknown\"\n\n[target.wasm32-unknown-unknown]\nrustflags = [\n    \"-C\", \"link-arg=-s\",\n    \"-C\", \"target-cpu=mvp\",\n    \"--cfg\", \"getrandom_backend=\\\"custom\\\"\",\n]\n",
        )?;
    } else {
        fs::write(dir.join("rust-toolchain.toml"), TEMPLATE_RUST_TOOLCHAIN)?;

        let cargo_dir = dir.join(".cargo");
        fs::create_dir_all(&cargo_dir)?;
        fs::write(cargo_dir.join("config.toml"), TEMPLATE_CARGO_CONFIG)?;
    }

    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir)?;
    fs::write(src_dir.join("lib.rs"), generate_lib_rs(method_names))?;
    fs::write(src_dir.join("contract.bin"), bytecode)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Build execution
// ---------------------------------------------------------------------------

fn build_wasm(project_dir: &Path, compat: bool) -> Result<PathBuf> {
    let mut args = vec!["build", "--release"];
    if compat {
        args.extend(["-Zbuild-std=std,panic_abort"]);
    }

    let output = Command::new("cargo")
        .args(&args)
        .current_dir(project_dir)
        .env_remove("RUSTUP_TOOLCHAIN")
        .output()
        .context("failed to run cargo build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("cargo build failed:\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",);
    }

    let wasm_path =
        project_dir.join("target/wasm32-unknown-unknown/release/monty_near_contract.wasm");

    if !wasm_path.exists() {
        bail!("WASM output not found at {}", wasm_path.display());
    }

    Ok(wasm_path)
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            input,
            output,
            compat,
            no_wasm_opt,
        } => {
            build_contract(&input, &output, compat, no_wasm_opt)?;
        }
    }

    Ok(())
}

fn build_contract(input: &Path, output: &Path, compat: bool, no_wasm_opt: bool) -> Result<()> {
    if compat {
        eprintln!("  Mode: compat (NearVM — nightly + -Zbuild-std -Ctarget-cpu=mvp)");
    }
    eprintln!("  Parsing {}...", input.display());
    let source =
        fs::read_to_string(input).with_context(|| format!("failed to read {}", input.display()))?;

    let method_names = find_exported_functions(&source)?;
    if method_names.is_empty() {
        bail!("no exported functions found (functions must not start with _)");
    }
    eprintln!(
        "  Found {} methods: {}",
        method_names.len(),
        method_names.join(", ")
    );

    eprint!("  Compiling...");
    let bytecode = precompile_contract(&source, &method_names)?;
    eprintln!(" {} bytes (single blob)", bytecode.len());

    eprintln!("  Building WASM...");
    let build_dir = std::env::current_dir()?.join("target/monty-near-build");
    if build_dir.exists() {
        let src_dir = build_dir.join("src");
        if src_dir.exists() {
            fs::remove_dir_all(&src_dir)?;
        }
    }
    fs::create_dir_all(&build_dir)?;

    write_project(&build_dir, &method_names, &bytecode, compat)?;

    let wasm_path = build_wasm(&build_dir, compat)?;

    let output_abs = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    fs::copy(&wasm_path, &output_abs)?;

    let raw_size = fs::metadata(&output_abs)?.len();

    if !no_wasm_opt {
        run_wasm_opt(&output_abs, compat, raw_size)?;
    }

    let final_size = fs::metadata(&output_abs)?.len();
    let size_kb = final_size as f64 / 1024.0;
    eprintln!();
    eprintln!("  \u{2713} {} ({:.0} KB)", output_abs.display(), size_kb);

    if compat {
        verify_no_bulk_memory(&output_abs)?;
    }

    Ok(())
}

fn run_wasm_opt(wasm_path: &Path, compat: bool, raw_size: u64) -> Result<()> {
    let wasm_str = wasm_path.display().to_string();
    let mut args = vec!["-Oz", &wasm_str, "-o", &wasm_str];

    // Default builds use post-MVP features that wasm-opt must be told about
    if !compat {
        args.extend([
            "--enable-bulk-memory",
            "--enable-nontrapping-float-to-int",
            "--enable-reference-types",
            "--enable-sign-ext",
        ]);
    }

    eprint!("  Optimizing with wasm-opt -Oz...");

    let output = Command::new("wasm-opt").args(&args).output();

    match output {
        Ok(result) if result.status.success() => {
            let opt_size = fs::metadata(wasm_path)?.len();
            let saved = raw_size.saturating_sub(opt_size);
            let pct = if raw_size > 0 {
                (saved as f64 / raw_size as f64) * 100.0
            } else {
                0.0
            };
            eprintln!(" saved {:.0} KB ({pct:.0}%)", saved as f64 / 1024.0);
            Ok(())
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            eprintln!(" failed");
            bail!("wasm-opt failed:\n{stderr}");
        }
        Err(_) => {
            eprintln!(
                " skipped (not found)\n    \
                 Install with: cargo install wasm-opt"
            );
            Ok(())
        }
    }
}

fn verify_no_bulk_memory(wasm_path: &Path) -> Result<()> {
    let output = Command::new("wasm-tools")
        .args([
            "validate",
            "--features=-bulk-memory",
            &wasm_path.display().to_string(),
        ])
        .output();

    match output {
        Ok(result) if result.status.success() => {
            eprintln!("  \u{2713} Verified: no bulk-memory instructions (NearVM compatible)");
            Ok(())
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            bail!(
                "compat build still contains bulk-memory instructions!\n\
                 wasm-tools validate failed:\n{stderr}\n\
                 This is a bug — please report it."
            );
        }
        Err(_) => {
            eprintln!(
                "  ! wasm-tools not found — skipping bulk-memory verification.\n    \
                 Install with: cargo install wasm-tools"
            );
            Ok(())
        }
    }
}
