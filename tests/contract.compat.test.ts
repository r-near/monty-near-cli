import { Near } from "near-kit"
import { Sandbox } from "near-kit/sandbox"
import { beforeAll, afterAll, test, expect, describe } from "bun:test"
import { readFileSync, existsSync } from "fs"
import { execSync } from "child_process"
import { resolve } from "path"

let sandbox: Sandbox
let near: Near
let contractId: string

const ROOT = resolve(import.meta.dir, "..")
const CLI_BIN = resolve(ROOT, "target/release/monty-near-cli")
const EXAMPLE_PY = resolve(ROOT, "examples/example.py")
const WASM_OUT = resolve(ROOT, "target/example_compat_test.wasm")

const encode = (s = "") => new TextEncoder().encode(s)

function decodeResult(outcome: any): string {
  const b64 = outcome?.status?.SuccessValue
  if (!b64) return ""
  return Buffer.from(b64, "base64").toString("utf-8")
}

function getLogs(outcome: any): string[] {
  return (outcome?.receipts_outcome ?? []).flatMap(
    (r: any) => r?.outcome?.logs ?? []
  )
}

beforeAll(async () => {
  execSync("cargo build --release", { cwd: ROOT, stdio: "inherit" })
  execSync(`${CLI_BIN} build ${EXAMPLE_PY} -o ${WASM_OUT} --compat`, {
    cwd: ROOT,
    stdio: "inherit",
  })
  if (!existsSync(WASM_OUT)) throw new Error(`WASM not found at ${WASM_OUT}`)

  // Use a stable release instead of master — compat mode targets production NearVM
  sandbox = await Sandbox.start({ version: "2.10.6" })
  const root = sandbox.rootAccount
  contractId = root.id

  near = new Near({ network: sandbox, defaultSignerId: contractId })

  const wasm = readFileSync(WASM_OUT)
  console.log(`WASM size (compat): ${(wasm.length / 1024).toFixed(0)} KB`)

  await near.transaction(root.id).deployContract(root.id, wasm).send()
  await near.call(contractId, "hello", {})
}, 120_000)

afterAll(async () => {
  if (sandbox) await sandbox.stop()
})

// ---------------------------------------------------------------------------
// Stateless tests
// ---------------------------------------------------------------------------

describe("views", () => {
  test("hello", async () => {
    const result = await near.view(contractId, "hello")
    console.log(`hello() → ${result}`)
    expect(result).toBe("Hello from Monty on NEAR!")
  })

  test("whoami", async () => {
    const result = (await near.view(contractId, "whoami")) as string
    console.log(`whoami() → ${result}`)
    expect(result).toContain(contractId)
    expect(result).toContain("at block")
  })
})

describe("stateless calls", () => {
  test("echo", async () => {
    const o = await near.call(contractId, "echo", encode("test data 123"))
    const result = decodeResult(o)
    console.log(`echo("test data 123") → ${result}`)
    expect(result).toBe("test data 123")
  })

  test("echo empty", async () => {
    const o = await near.call(contractId, "echo", encode())
    const result = decodeResult(o)
    console.log(`echo("") → ${result || "(empty)"}`)
    expect(result).toBe("")
  })

  test("greet with name", async () => {
    const o = await near.call(contractId, "greet", encode("Alice"))
    const result = decodeResult(o)
    console.log(`greet("Alice") → ${result}`)
    expect(result).toBe("Hello, Alice!")
  })

  test("greet default", async () => {
    const o = await near.call(contractId, "greet", encode())
    const result = decodeResult(o)
    console.log(`greet("") → ${result}`)
    expect(result).toBe("Hello, World!")
  })

  test("caller_info", async () => {
    const o = await near.call(contractId, "caller_info", {})
    const result = decodeResult(o)
    console.log(`caller_info() → ${result}`)
    expect(result).toContain(`predecessor=${contractId}`)
    expect(result).toContain(`signer=${contractId}`)
    expect(result).toContain("block=")
    expect(result).toContain("timestamp=")
  })

  test("hash_it", async () => {
    const o = await near.call(contractId, "hash_it", encode("hello"))
    const result = decodeResult(o)
    console.log(`hash_it("hello") → ${result}`)
    const expectedSha = new Bun.CryptoHasher("sha256")
      .update("hello")
      .digest("hex")
    expect(result).toContain(`sha256=${expectedSha}`)
    expect(result).toContain("keccak256=")
  })

  test("log_and_return", async () => {
    const o = await near.call(
      contractId, "log_and_return", encode("important event"),
    )
    const result = decodeResult(o)
    const logs = getLogs(o)
    console.log(`log_and_return("important event") → ${result}`)
    for (const log of logs) console.log(`  ↳ ${log}`)
    expect(result).toBe("logged: important event")
    expect(logs.some((l) => l.includes("LOG: important event"))).toBe(true)
  })

  test("log_and_return default", async () => {
    const o = await near.call(contractId, "log_and_return", encode())
    const result = decodeResult(o)
    console.log(`log_and_return("") → ${result}`)
    expect(result).toBe("logged: default log message")
  })
})

// ---------------------------------------------------------------------------
// Stateful tests (order matters)
// ---------------------------------------------------------------------------

describe("counter", () => {
  test("increments", async () => {
    const o1 = await near.call(contractId, "counter", {})
    const v1 = parseInt(decodeResult(o1))

    const o2 = await near.call(contractId, "counter", {})
    const v2 = parseInt(decodeResult(o2))
    console.log(`counter: ${v1} → ${v2} (incremented)`)
    expect(v2).toBe(v1 + 1)
  })

  test("get_counter reads without incrementing", async () => {
    const o = await near.call(contractId, "counter", {})
    const expected = decodeResult(o)

    // Force another block so the view sees the latest state
    await near.call(contractId, "hello", {})

    const v1 = await near.view(contractId, "get_counter")
    const v2 = await near.view(contractId, "get_counter")
    console.log(`counter() wrote ${expected}, get_counter() reads ${v1} (stable: ${v1 === v2})`)
    expect(String(v1)).toBe(expected)
    expect(String(v2)).toBe(expected)
  })
})

describe("storage", () => {
  test("set_get", async () => {
    const o = await near.call(contractId, "set_get", encode("hello storage"))
    const result = decodeResult(o)
    console.log(`set_get("hello storage") → ${result}`)
    expect(result).toBe("hello storage")
  })

  test("remove_key", async () => {
    await near.call(contractId, "set_get", encode("temp_value"))
    const o = await near.call(contractId, "remove_key", {})
    const result = decodeResult(o)
    console.log(`remove_key() → ${result}`)
    expect(result).toBe("removed")
  })

  test("kv_put", async () => {
    const o = await near.call(contractId, "kv_put", encode("mycolor:blue"))
    const result = decodeResult(o)
    console.log(`kv_put("mycolor:blue") → ${result}`)
    expect(result).toBe("ok")
  })

  test("kv_put error", async () => {
    const o = await near.call(contractId, "kv_put", encode("no_colon_here"))
    const result = decodeResult(o)
    console.log(`kv_put("no_colon_here") → ${result}`)
    expect(result).toBe("error: expected key:value")
  })

  test("kv_get", async () => {
    await near.call(contractId, "kv_put", encode("fruit:apple"))
    const o = await near.call(contractId, "kv_get", encode("fruit"))
    const result = decodeResult(o)
    console.log(`kv_get("fruit") → ${result}`)
    expect(result).toBe("apple")
  })

  test("kv_get missing", async () => {
    const o = await near.call(contractId, "kv_get", encode("nonexistent_key_xyz"))
    const result = decodeResult(o)
    console.log(`kv_get("nonexistent_key_xyz") → ${result || "(empty)"}`)
    expect(result).toBe("")
  })

  test("kv_round_trip", async () => {
    const pairs = [
      ["name", "monty"],
      ["version", "1.0"],
      ["lang", "python"],
    ]
    for (const [k, v] of pairs) {
      await near.call(contractId, "kv_put", encode(`${k}:${v}`))
    }
    for (const [k, v] of pairs) {
      const o = await near.call(contractId, "kv_get", encode(k))
      const result = decodeResult(o)
      console.log(`kv_get("${k}") → ${result}`)
      expect(result).toBe(v)
    }
  }, 30_000)
})
