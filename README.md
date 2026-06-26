# sqlite-loader-wit

Canonical WIT contract for SQLite WebAssembly extensions. Defines the interface a `.wasm` extension implements so it can be loaded into a SQLite host — whether that host is native SQLite + a WASM runtime ([`sqlite-wasm-loader`](https://github.com/tegmentum/sqlite-wasm-loader)) or SQLite-compiled-to-WASM ([`sqlite-wasm`](https://github.com/tegmentum/sqlite-wasm)).

Vendored into both consumers as a git submodule.

## Package

```
sqlite:extension@1.0.0
```

## Files

| File           | Contents                                                              |
|----------------|-----------------------------------------------------------------------|
| `types.wit`    | Shared types: `sql-value` (variant), error, enums, records           |
| `host-spi.wit` | Host-imported SPIs: `spi`, `prepared`, `transaction`, `schema`, `logging`, `config`, `state`, `cache`, `random`, `text`, `hashing`, `encoding` |
| `guest.wit`    | Guest-exported interfaces: `metadata`, `lifecycle`, `scalar-function`, `aggregate-function`, `collation`, `authorizer`, `update-hook`, `commit-hook` |
| `world.wit`    | Six capability-graded worlds: `minimal`, `stateful`, `lifecycle-aware`, `authorizing`, `hooked`, `full` |

## Design

**Declarative registration.** Extensions don't imperatively call `register-scalar-function(...)` on the host. They export a single `metadata.describe()` that returns a `manifest` listing every scalar function, aggregate, and collation they provide, with guest-assigned IDs. The host calls `describe()` once after instantiation and wires everything up. The host then dispatches by calling back into the corresponding interface (`scalar-function.call(func-id, args)`, `aggregate-function.step(func-id, context-id, args)`, etc.).

**Variant `sql-value`.** SQL values use the idiomatic component-model `variant`:

```wit
variant sql-value {
    null,
    integer(s64),
    real(f64),
    text(string),
    blob(list<u8>),
    wit-value(wit-value-payload),
}
```

The `wit-value` arm (new in `@1.0.0`) carries a canonical-CBOR-encoded WIT record together with a 32-byte `canon:wit` shape hash and a symbolic name. Used by record-typed shim functions; hosts decode via wasm-side imports declared in the manifest's `typed-values` field. See [`MIGRATION-1.0.md`](MIGRATION-1.0.md).

**Capability-graded worlds.** Pick the smallest world that fits the extension. A pure scalar function uses `minimal`. A stateful aggregate uses `stateful`. Mix in `lifecycle`, `authorizer`, or `hooks` as needed. Use `full` only when you genuinely need the kitchen sink.

## Versioning

`@1.0.0` is the first stable contract version. Subsequent breaking changes follow semver: incompatible WIT shape changes bump the MAJOR; additive interface members bump the MINOR. The contract surface — the union of all interfaces and worlds in `wit/` — is what the version covers.

See [`MIGRATION-1.0.md`](MIGRATION-1.0.md) for the `@0.1.0` → `@1.0.0` upgrade guide.

## License

MIT
