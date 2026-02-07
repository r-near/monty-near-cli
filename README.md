# monty-near-cli

Compile Python smart contracts to NEAR-deployable WASM using [Monty](https://github.com/pydantic/monty).

## Quick start

```bash
# Install
cargo install --path .

# Compile a Python contract to WASM
monty-near-cli build contract.py -o contract.wasm

# Deploy to NEAR
near deploy myaccount.testnet contract.wasm
near call myaccount.testnet hello --accountId myaccount.testnet
```

Rust 1.91.0 and the `wasm32-unknown-unknown` target are installed automatically via `rust-toolchain.toml`.

### Compatibility mode (current testnet/mainnet)

The default build targets the upcoming Wasmtime-based runtime (nearcore 2.12+). To deploy to **current** testnet or mainnet (which still use NearVM), use `--compat`:

```bash
monty-near-cli build contract.py -o contract.wasm --compat
```

This uses nightly Rust with `-Zbuild-std` and `-Ctarget-cpu=mvp` to produce WASM without bulk-memory instructions that NearVM rejects. Requires `rustup toolchain install nightly` and the `rust-src` component (installed automatically via the generated `rust-toolchain.toml`). The CLI itself remains on stable Rust.

If [`wasm-tools`](https://github.com/bytecodealliance/wasm-tools) is installed, the build automatically verifies the output contains no bulk-memory instructions.

### Build flags

| Flag | Effect |
|------|--------|
| `--compat` | Build for current production NearVM (nightly + `-Zbuild-std -Ctarget-cpu=mvp`) |
| `--no-wasm-opt` | Skip `wasm-opt -Oz` post-processing (enabled by default if `wasm-opt` is in PATH) |
| `-o <path>` | Output path (default: `contract.wasm`) |

## Example contract

```python
def hello():
    value_return("Hello from Python on NEAR!")

def counter():
    count = storage_read("count")
    if count is None:
        count = 0
    else:
        count = int(count)
    count = count + 1
    storage_write("count", str(count))
    value_return(str(count))
```

Every top-level `def` becomes an exported NEAR contract method. Functions starting with `_` are private helpers. All [NEAR host functions](https://docs.near.org/build/smart-contracts/anatomy/environment) are available as Python builtins — no imports needed.

See [`examples/example.py`](examples/example.py) for a contract exercising the core host functions.

## What is Monty?

[Monty](https://github.com/pydantic/monty) is a Python-to-Rust compiler by the Pydantic team. It takes a subset of Python, parses it with [ruff](https://github.com/astral-sh/ruff)'s parser, and compiles it to a custom bytecode format. That bytecode runs on a small Rust VM (`MontyRun`) that can be compiled to `wasm32-unknown-unknown` — making it suitable for embedding in NEAR smart contracts.

This CLI automates the pipeline: parse Python source → compile to Monty bytecode → embed the bytecode in a Rust WASM project with a NEAR-compatible runtime → produce a deployable `.wasm` file.

## How compilation works

```
Python source → monty-near-cli (host) → contract.wasm (deployable)
```

1. **Parse** — find all top-level `def` functions in the Python file.
2. **Compile** — compile the entire source plus a generated dispatcher into a single Monty bytecode blob using `MontyRun::new()` + `.dump()`. The dispatcher is an `if`/`elif` chain that routes a `_method` variable to the correct function.
3. **Scaffold** — create a temporary Rust project in `target/monty-near-build/` using embedded templates (`Cargo.toml`, `lib.rs`, toolchain config).
4. **Splice** — inject the serialized bytecode and `#[no_mangle] pub extern "C" fn` exports into the template's `lib.rs` at marker comments.
5. **Build** — `cargo build --release` targeting `wasm32-unknown-unknown`. LTO strips the Python parser entirely; only the VM and bytecode remain.
6. **Optimize** — run `wasm-opt -Oz` on the output for size reduction (~11-12% savings).
7. **Verify** — in `--compat` mode, run `wasm-tools validate --features=-bulk-memory` to confirm the output is NearVM-safe.

Each exported method deserializes the shared bytecode, passes the method name as an input variable to the VM, and the dispatcher routes execution to the correct Python function.

## Testing

Integration tests use [bun](https://bun.sh) and [near-kit](https://kit.near.tools) to deploy the compiled contract to a local NEAR sandbox:

```bash
cd tests && bun install && bun test
```

There are two test suites:

- **`contract.test.ts`** — default build, runs against sandbox `master` (Wasmtime with bulk-memory support)
- **`contract.compat.test.ts`** — `--compat` build, runs against sandbox `2.10.6` (production NearVM)

To run just the compat tests: `bun test contract.compat.test.ts`

## Technical details

### Bulk memory and NearVM compatibility

Starting with Rust 1.87, LLVM emits `bulk-memory` WASM instructions (`memory.copy`, `memory.fill`) by default. NEAR's current production VM (NearVM, a Wasmer 2.x fork) rejects these with `PrepareError::Instantiate`, so most NEAR contract tooling is pinned to Rust 1.86 or earlier.

This project can't pin to Rust 1.86 because Monty's dependencies require `let_chains` (stable since 1.87). Instead, two build modes are provided:

| | Default | `--compat` |
|---|---|---|
| **Target runtime** | Wasmtime (nearcore 2.12+) | Current NearVM (Wasmer) |
| **Rust toolchain** | Stable 1.91.0 | Nightly |
| **Bulk-memory instructions** | Present | Stripped via `-Ctarget-cpu=mvp` |
| **Deployable today** | Sandbox only | Testnet and mainnet |
| **Cargo flags** | `build --release` | `build --release -Zbuild-std=std,panic_abort` |

**⚠️ The default build (without `--compat`) is not yet deployable on testnet or mainnet.** The Wasmtime switch is part of nearcore 2.12, with mainnet deployment expected late March / early April 2026 ([stabilization PR](https://github.com/near/nearcore/pull/14315)). Until then, use `--compat` for testnet/mainnet, or deploy to a local sandbox running `master`.

For background, see the [`contract-runtime > bulk memory support`](https://near.zulipchat.com/#narrow/channel/295306-contract-runtime/topic/bulk.20memory.20support) thread on near.zulipchat.com.

### wasm-opt

The build runs [`wasm-opt -Oz`](https://github.com/WebAssembly/binaryen) automatically after `cargo build` to reduce WASM size through dead code elimination, constant folding, and other optimizations. This typically saves ~11-12% (~100 KB). Pass `--no-wasm-opt` to skip this step, or install wasm-opt with `cargo install wasm-opt` if it's not already available.

Note: while `wasm-opt` can strip some post-MVP features like `multi-value` and `reference-types`, it [cannot strip `bulk-memory` instructions](https://near.zulipchat.com/#narrow/channel/295306-contract-runtime/topic/bulk.20memory.20support). This is why `--compat` solves the problem at the compiler level (via `-Ctarget-cpu=mvp`) rather than relying on post-processing.

### getrandom and ahash

Monty depends on `ahash`, which depends on `getrandom` for hash randomization. `getrandom` doesn't compile for `wasm32-unknown-unknown` by default. Instead of using the `no-rng` feature flag (which would require forking monty's `Cargo.toml`), the template project implements a [getrandom 0.3 custom backend](https://docs.rs/getrandom/latest/getrandom/#custom-backend) that provides randomness from NEAR's VRF-based `random_seed()` host function.

## Known limitations

- **Python subset** — Monty compiles a subset of Python. Classes, decorators, exceptions (`try`/`except`), list comprehensions, `*args`/`**kwargs`, and the standard library are not supported. See [Monty's documentation](https://github.com/pydantic/monty) for the full list of supported features.
- **String-only storage** — host functions pass data as strings. There is no built-in JSON serialization; parse and format manually.
- **No panic handling** — if the Monty VM encounters an error, the contract panics with a generic message. Python exceptions are not supported.
- **WASM size** — the output is ~790-830 KB (after wasm-opt) due to the embedded Monty VM. This is within NEAR's 1.5 MB contract size limit but larger than typical Rust SDK contracts.

## Project structure

```
monty-near-cli/
├── src/main.rs                # CLI: parse → compile → scaffold → build → optimize
├── template/
│   ├── Cargo.toml             # Generated project dependencies
│   ├── rust-toolchain.toml    # Pins Rust 1.91.0 + wasm32 target
│   ├── .cargo/config.toml     # WASM target, getrandom backend
│   └── src/lib.rs             # NEAR runtime: FFI imports, host wrappers, VM loop
├── examples/
│   └── example.py             # 13-method contract using all host functions
├── tests/
│   ├── contract.test.ts       # Integration tests (bun + near-kit, sandbox master)
│   ├── contract.compat.test.ts # Compat mode tests (sandbox 2.10.6)
│   └── package.json
└── Cargo.toml
```

## License

MIT
