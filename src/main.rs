use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use monty::MontyRun;

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
    },
}

// ---------------------------------------------------------------------------
// External NEAR functions available to Python contracts
// ---------------------------------------------------------------------------

fn near_external_functions() -> Vec<String> {
    [
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
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ---------------------------------------------------------------------------
// Python source parsing — find exported top-level functions
// ---------------------------------------------------------------------------

/// Find top-level function names that don't start with `_`.
fn find_exported_functions(source: &str) -> Vec<String> {
    let mut functions = Vec::new();
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("def ") {
            let rest = rest.trim_start();
            if let Some(paren_pos) = rest.find('(') {
                let name = rest[..paren_pos].trim();
                if !name.is_empty()
                    && !name.starts_with('_')
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                {
                    functions.push(name.to_string());
                }
            }
        }
    }
    functions
}

// ---------------------------------------------------------------------------
// Pre-compilation — compile each method's Python source to Monty bytecode
// ---------------------------------------------------------------------------

struct CompiledMethod {
    name: String,
    bytecode: Vec<u8>,
}

fn precompile_method(full_source: &str, method_name: &str) -> Result<CompiledMethod> {
    let program = format!("{full_source}\n\n{method_name}()\n");
    let external_functions = near_external_functions();

    let runner = MontyRun::new(program, "contract.py", vec![], external_functions)
        .map_err(|e| anyhow::anyhow!("compilation failed for '{method_name}': {e}"))?;

    let bytecode = runner
        .dump()
        .context(format!("serialization failed for '{method_name}'"))?;

    Ok(CompiledMethod {
        name: method_name.to_string(),
        bytecode,
    })
}

// ---------------------------------------------------------------------------
// Code generation — splice generated code into the template
// ---------------------------------------------------------------------------

/// Generate the `lib.rs` source by inserting bytecode statics and export
/// functions into the template at the marker comments.
fn generate_lib_rs(methods: &[CompiledMethod]) -> String {
    let mut bytecode_statics = String::new();
    let mut exports = String::new();

    for m in methods {
        let upper = m.name.to_uppercase();

        bytecode_statics.push_str(&format!(
            "static {upper}_BYTECODE: &[u8] = include_bytes!(\"{name}.bin\");\n",
            name = m.name,
        ));

        exports.push_str(&format!(
            "#[no_mangle]\npub extern \"C\" fn {name}() {{\n    run_precompiled({upper}_BYTECODE);\n}}\n\n",
            name = m.name,
        ));
    }

    TEMPLATE_LIB_RS
        .replace(MARKER_BYTECODE, &bytecode_statics)
        .replace(MARKER_EXPORTS, &exports)
}

// ---------------------------------------------------------------------------
// Project scaffolding — write the temporary Rust project to disk
// ---------------------------------------------------------------------------

fn write_project(dir: &Path, methods: &[CompiledMethod]) -> Result<()> {
    fs::write(dir.join("Cargo.toml"), TEMPLATE_CARGO_TOML)?;
    fs::write(dir.join("rust-toolchain.toml"), TEMPLATE_RUST_TOOLCHAIN)?;

    let cargo_dir = dir.join(".cargo");
    fs::create_dir_all(&cargo_dir)?;
    fs::write(cargo_dir.join("config.toml"), TEMPLATE_CARGO_CONFIG)?;

    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir)?;
    fs::write(src_dir.join("lib.rs"), generate_lib_rs(methods))?;

    for m in methods {
        fs::write(src_dir.join(format!("{}.bin", m.name)), &m.bytecode)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Build execution
// ---------------------------------------------------------------------------

fn build_wasm(project_dir: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["build", "--release"])
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
        Commands::Build { input, output } => {
            build_contract(&input, &output)?;
        }
    }

    Ok(())
}

fn build_contract(input: &Path, output: &Path) -> Result<()> {
    eprintln!("  Parsing {}...", input.display());
    let source =
        fs::read_to_string(input).with_context(|| format!("failed to read {}", input.display()))?;

    let method_names = find_exported_functions(&source);
    if method_names.is_empty() {
        bail!("no exported functions found (functions must not start with _)");
    }
    eprintln!(
        "  Found {} methods: {}",
        method_names.len(),
        method_names.join(", ")
    );

    let mut methods = Vec::new();
    for name in &method_names {
        eprint!("  Pre-compiling {name}...");
        let compiled = precompile_method(&source, name)?;
        eprintln!(" {} bytes", compiled.bytecode.len());
        methods.push(compiled);
    }

    eprintln!("  Building WASM...");
    let build_dir = std::env::current_dir()?.join("target/monty-near-build");
    if build_dir.exists() {
        let src_dir = build_dir.join("src");
        if src_dir.exists() {
            fs::remove_dir_all(&src_dir)?;
        }
    }
    fs::create_dir_all(&build_dir)?;

    write_project(&build_dir, &methods)?;

    let wasm_path = build_wasm(&build_dir)?;

    let output_abs = if output.is_absolute() {
        output.to_path_buf()
    } else {
        std::env::current_dir()?.join(output)
    };
    fs::copy(&wasm_path, &output_abs)?;

    let size = fs::metadata(&output_abs)?.len();
    let size_kb = size as f64 / 1024.0;
    eprintln!();
    eprintln!("  \u{2713} {} ({:.0} KB)", output_abs.display(), size_kb);

    Ok(())
}
