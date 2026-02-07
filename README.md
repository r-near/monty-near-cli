# monty-near-cli

Compile Python smart contracts to NEAR-deployable WASM — using [Monty](https://github.com/pydantic/monty), stable Rust, and zero forks.

## Highlights

- **Zero forks** — uses upstream Monty directly (`pydantic/monty` main branch). No patched ruff, no patched nearcore.
- **Stable Rust 1.91** — no nightly toolchain, no `-Zbuild-std`, no `--ignore-rust-version`.
- **No post-processing** — raw `cargo build --release` output deploys directly. No `wasm-opt` required.
- **Protocol 84 native** — NEAR's Wasmtime runtime handles bulk-memory WASM instructions (`memory.copy`/`memory.fill`) natively.
- **14 NEAR host functions** — full coverage of the contract API surface.

## How it works

```
┌─────────────┐     ┌───────────────────┐     ┌──────────────────┐
│  Python src ├────►│  monty-near-cli   ├────►│  contract.wasm   │
│  (*.py)     │     │  (host machine)   │     │  (deployable)    │
└─────────────┘     └───────────────────┘     └──────────────────┘
```

1. **Parse** — the CLI reads your Python file and finds all top-level `def` functions (functions starting with `_` are ignored).
2. **Pre-compile** — each function is compiled to Monty bytecode on the host using `MontyRun::new()` + `.dump()`.
3. **Scaffold** — a temporary Rust project is created in `target/monty-near-build/` using embedded templates.
4. **Splice** — serialized bytecode statics and `#[no_mangle] pub extern "C" fn` exports are spliced into the template at marker comments.
5. **Build** — `cargo build --release` produces the final `wasm32-unknown-unknown` binary.
6. **Output** — the WASM is copied to your specified output path.

The resulting WASM contains only the Monty VM and serialized bytecode — LTO strips the parser entirely.

## Installation

```bash
cargo install --path .
```

Rust 1.91.0 is installed automatically via `rust-toolchain.toml`.

## Usage

```bash
monty-near-cli build contract.py -o contract.wasm
```

## Writing contracts

Contract methods are plain Python functions. Each top-level `def` becomes a NEAR contract method. The following built-in functions are available without any imports:

### I/O

| Function | Description |
|---|---|
| `input()` | Read the call's input data as a string |
| `value_return(data)` | Set the return value of the call |
| `log(message)` | Emit a log visible in transaction receipts |

### Storage

| Function | Description |
|---|---|
| `storage_write(key, value)` | Write a key-value pair to persistent storage |
| `storage_read(key)` | Read a value by key (returns `None` if missing) |
| `storage_remove(key)` | Delete a key from storage |
| `storage_has_key(key)` | Check if a key exists (returns `bool`) |

### Context

| Function | Description |
|---|---|
| `current_account_id()` | The contract's own account ID |
| `predecessor_account_id()` | The immediate caller's account ID |
| `signer_account_id()` | The original transaction signer |
| `block_height()` | Current block height |
| `block_timestamp()` | Current block timestamp (nanoseconds) |

### Cryptography

| Function | Description |
|---|---|
| `sha256(data)` | SHA-256 hash, returned as hex string |
| `keccak256(data)` | Keccak-256 hash, returned as hex string |

### Conventions

- Top-level `def` functions become contract methods.
- Functions starting with `_` are private (not exported).
- `input()` returns raw UTF-8 bytes as a string — parse as needed.
- `value_return()` accepts a string and sets it as the call's return value.
- `storage_read()` returns `None` if the key doesn't exist.

## Example

```python
def hello():
    value_return("Hello from Monty on NEAR!")

def greet():
    name = input()
    if name == "":
        name = "World"
    value_return("Hello, " + name + "!")

def counter():
    count = storage_read("count")
    if count is None:
        count = 0
    else:
        count = int(count)
    count = count + 1
    storage_write("count", str(count))
    value_return(str(count))

def kv_put():
    data = input()
    pos = data.find(":")
    if pos < 0:
        value_return("error: expected key:value")
    else:
        key = data[0:pos]
        val = data[pos + 1:]
        storage_write(key, val)
        value_return("ok")

def kv_get():
    key = input()
    val = storage_read(key)
    if val is None:
        value_return("")
    else:
        value_return(val)
```

Build and deploy:

```bash
monty-near-cli build examples/example.py -o contract.wasm
near deploy myaccount.testnet contract.wasm
near call myaccount.testnet hello --accountId myaccount.testnet
```

See [`examples/example.py`](examples/example.py) for a comprehensive contract exercising all 14 host functions.

## Testing

Integration tests deploy the compiled contract to a NEAR sandbox and exercise every method:

```bash
cargo test --test sandbox
```

This uses [`near-workspaces`](https://github.com/near/near-workspaces-rs) to automatically download and run a sandbox node. No manual setup required.

## Why no forks?

Previously, building Monty for NEAR required maintaining forks of multiple repositories. All of those patches are now unnecessary:

| What was forked | Why | Why it's no longer needed |
|---|---|---|
| `pydantic/monty` | `let_chains`, `.cast_signed()`, `.is_multiple_of()` were unstable | All stabilized in Rust 1.87+ |
| `pydantic/monty` | ahash needed `no-rng` feature | getrandom 0.3 custom backend handles it |
| `astral-sh/ruff` | One `let_chains` usage in `helpers.rs` | Stable since Rust 1.87 |
| Build flags | `-Ctarget-cpu=mvp`, `-Zbuild-std` to strip bulk-memory | Protocol 84 uses Wasmtime — bulk-memory supported natively |
| `wasm-opt` | Required post-processing for old protocol | Raw WASM deploys directly |

## Project structure

```
monty-near-cli/
├── src/main.rs              # CLI: parse → compile → scaffold → build
├── template/
│   ├── Cargo.toml            # Generated project manifest
│   ├── rust-toolchain.toml   # Pins Rust 1.91.0 + wasm32 target
│   ├── .cargo/config.toml    # WASM target config, getrandom backend
│   └── src/lib.rs            # NEAR runtime: FFI, host wrappers, VM loop
├── examples/
│   └── example.py            # 13-method contract using all host functions
├── tests/
│   └── sandbox.rs            # Integration tests via near-workspaces
├── Cargo.toml
└── rust-toolchain.toml       # CLI's own toolchain (1.91.0)
```

## Requirements

- Rust 1.91.0+ (auto-installed via `rust-toolchain.toml`)
- That's it.

## License

MIT
