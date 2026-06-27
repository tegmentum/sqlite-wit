# Migrating to `sqlite:extension@1.0.0`

The `sqlite:extension` WIT contract bumps from `@0.1.0` to `@1.0.0` to land
two related additions described in
[`PLAN-wit-value-extension.md`][plan]:

1. A `wit-value` arm on the `sql-value` variant for ferrying typed WIT
   records through the SQL value pipeline.
2. A `typed-values` field on the `metadata.manifest` record listing
   per-record decoder/encoder import bindings.

Both changes are **structurally additive**: existing arms of `sql-value`
keep their discriminant tags, and the new manifest field defaults to an
empty list when absent. Per the WebAssembly Component Model however,
*any* shape change to an exported variant is treated as a MAJOR by the
canonical ABI bytes  hence the `0.1.0  1.0.0` bump rather than a
patch or minor.

## What changed

### `types.wit`  `sql-value` variant

```diff
 variant sql-value {
     null,
     integer(s64),
     real(f64),
     text(string),
     blob(list<u8>),
+    wit-value(wit-value-payload),
 }
+
+record wit-value-payload {
+    type-id: list<u8>,        // 32-byte sha256 canon:wit shape hash
+    bytes: list<u8>,          // canonical-CBOR encoding
+    symbolic-name: string,    // human-readable diagnostic name
+}
```

The `wit-value` arm is reserved for record-typed shim functions
(PostGIS + MobilityDB families). Extensions that only exchange the
classic scalar arms (null/integer/real/text/blob) never construct a
`wit-value` and can ignore it.

### `guest.wit`  `metadata.manifest` record

```diff
 record manifest {
     name: string,
     version: string,
     scalar-functions: list<scalar-function-spec>,
     ...
+    typed-values: list<typed-value-binding>,
 }
+
+record typed-value-binding {
+    type-id: list<u8>,
+    symbolic-name: string,
+    decoder-import: string,
+    encoder-import: string,
+}
```

Codegen-emitted bridges (PostGIS today, MobilityDB Phase E) populate
`typed-values` with one entry per record shape they marshal. The
host registers each entry into its per-extension typed-value registry
at extension-init time so per-call lookups are O(1) by `type-id`.

Hand-written extensions whose manifests have no record-typed functions
return an empty list and never see the `wit-value` machinery.

## What extension authors need to do

### Hand-authored extensions (no record-typed params/returns)

**Rebuild against `@1.0.0`. No code changes required.**

The wit-value arm is additive at the source level; existing match
arms on `SqlValue::{Null, Integer, Real, Text, Blob}` stay
exhaustive once you add an `_` (or `SqlValue::WitValue(_)`) arm  the
Rust compiler will tell you which match expressions need that arm
when you recompile.

Typical fix in extension code:

```rust
match value {
    SqlValue::Null => ...,
    SqlValue::Integer(i) => ...,
    SqlValue::Real(r) => ...,
    SqlValue::Text(s) => ...,
    SqlValue::Blob(b) => ...,
+   SqlValue::WitValue(_) => Err("this extension does not accept wit-value args".into()),
}
```

The manifest's new `typed-values` field needs to be initialized when
building `Manifest { ... }` literals. Hand-authored extensions set it
to the empty list:

```rust
Manifest {
    name: "...".into(),
    version: "...".into(),
    // ...existing fields...
+   typed_values: Vec::new(),
}
```

### Codegen-emitted bridges (record-typed surfaces)

**Phase A lands the contract only.** Codegen-emitted bridges
(`postgis-sqlink-bridge` today; `mobilitydb-sqlink-bridge` Phase E)
keep emitting `typed_values: Vec::new()` for now. Phase C of the
plan teaches `sqlink-shim-codegen` to populate the field with one
entry per record-typed shape and to emit dispatch arms that
construct `SqlValue::WitValue(...)` on call/return.

End-user authors of these bridges do not need to do anything
manually  the codegen regenerates the bridge crate when run.

### Loader and host crates

`sqlink-host`, `sqlink-loader`, and `composed-cli` bind against the
contract via `wit-bindgen::generate!{ path: "../../sqlite-loader-
wit/wit", ... }` and pick up the variant addition automatically on
recompile. Phase B of the plan teaches each host to actually decode
`wit-value` (look up the type-id, call the wasm-side decoder, hand
the record to the called function); Phase A is satisfied by the
hosts just refusing the arm at the runtime gate
(`unimplemented for sql-value::wit-value, will land in Phase B`).

## Backwards compatibility

Components compiled against `sqlite:extension@0.1.0` are **rejected**
by hosts speaking `@1.0.0` — Phase F of `PLAN-wit-value-extension`
([#485 Phase 2][issue485]) landed the loader pre-check. The two
contract versions are not ABI-compatible: the canonical-ABI byte
sequence for `sql-value` differs (new variant tag, new payload arm).

### What an unmigrated `@0.x` extension sees against a `@1.x` host

The pre-check runs before `Component::new` / `instantiate_async` and
emits a model-level error — not a cryptic wasmtime trap — through
every loader in the matrix:

* **`sqlink-host` native binary**: load fails with
  ```
  extension '<name>' targets sqlite:extension contract 0.x but this host
  speaks contract 1.x; rebuild it against the current WIT (or use the
  matching host version)
  ```
  Wire-checked in `host/src/lib.rs::contract_guard_tests` (5 cases) and
  surfaced through `sqlink --contract-version` (prints the host's major).
* **`sqlink-loader.dylib`**: vanilla SQLite's `SELECT
  load_extension('./sqlink-loader.so')` path inherits the same guard
  via `Host::load_extension` and surfaces the same friendly message.
  Tests: `sqlink-loader::load::tests::loader_path_rejects_*` (2 cases).
* **`composed-cli-worker` (browser)**: jco's runtime-bindgen has no
  equivalent reject-on-instantiate, so the worker pre-screens each
  component's bytes via `src/contract-guard.js` (regex over the
  component-model binary's import names) and throws the same error
  through the `postMessage` response. Tests:
  `browser/tests/contract-guard.test.mjs` (10 cases). The worker also
  responds to a `contractVersion` message so test pages can introspect
  which contract the embedded composed-cli speaks — analogous to
  `sqlink --contract-version` on the native side.

A legacy/unversioned component (no `sqlite:extension/...@MAJOR` import
at all — a pre-versioning artifact) is rejected with the same message
shape but the major reads as `UNVERSIONED` to make the diagnostic
unambiguous.

In the interim (between Phase A and Phase F), hosts loading an old
`@0.1.0` component may see undefined behavior when `sql-value` flows
across the boundary. The recommended migration path is mechanical:

1. Pull the latest `sqlite-loader-wit` submodule on each extension's
   workspace.
2. `cargo clean && cargo build --release --target wasm32-wasip2` (the
   wit-bindgen cache won't pick up the contract change without a
   clean).
3. Republish the resulting `.wasm`.

Each Tegmentum-owned bridge is regenerated alongside the contract
bump as part of Phase A's catalog regen step.

## Wire format

The `wit-value-payload.bytes` field uses **canonical CBOR** per the
`canon:cbor` profile from
[`PLAN-orchestration-integration.md` (#486)][issue486]. Specifically:

- Deterministic field ordering (lexicographic by name for records).
- Length-delimited rather than indefinite-length items.
- Canonical integer encoding (smallest representation that fits).
- No tags except where required by the WIT shape ↔ CBOR mapping.

Decoder/encoder import names follow the convention:

```
<package>:wasm/serde-ops/<type-name>-from-canon-cbor
<package>:wasm/serde-ops/<type-name>-to-canon-cbor
```

Examples (planned for Phase C / E):

```
mobilitydb:wasm/serde-ops/tfloat-sequence-from-canon-cbor
mobilitydb:wasm/serde-ops/tfloat-sequence-to-canon-cbor
postgis:wasm/serde-ops/geometry-from-canon-cbor
postgis:wasm/serde-ops/geometry-to-canon-cbor
```

The codegen emits a thin wasm-side wrapper per record type
(~10 lines) that wraps `serde_cbor::de::from_slice` /
`serde_cbor::ser::to_vec_packed` against the bindgen-generated
record struct. Round-trip stability is a conformance test on the
canonical CBOR profile (#486 Tier 3, C3).

## Type identity (`type-id`)

`type-id` is the 32-byte sha256 of

```
"witcanon:1" || canonical-CBOR(WIT record shape)
```

per `canon:wit` normalization (#486 Tier 2). The hash is stable
across cosmetic WIT changes (whitespace, doc comments, field
ordering) and changes immediately on any structural change (renames,
type changes, additions, deletions).

The matching `symbolic-name` is the qualified record name in
`<package>:wasm/<interface>@<version>/<type-name>` form. Hosts use
this in error messages but **never** for matching  the hash is
authoritative. The codegen emits both fields together as a pair so
diagnostics still surface a useful name when a payload's `type-id`
doesn't match any registered binding.

## References

- [`PLAN-wit-value-extension.md`][plan]  the design + phasing.
- [`PLAN-wit-contract-versioning.md`][issue485]  the broader
  contract-versioning workstream Phase A folds in (#485 Phase 1).
- [`PLAN-orchestration-integration.md`][issue486]  the substrate
  the wire format and type-id hashing live in (#486 Tier 2 + 3).

[plan]: ../docs/plans/PLAN-wit-value-extension.md
[issue485]: ../docs/plans/PLAN-wit-contract-versioning.md
[issue486]: ../docs/plans/PLAN-orchestration-integration.md
