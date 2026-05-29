# CrustDB Project Spec

## Product Intent

CrustDB is a Python-first embedded database prototype with a Rust storage engine.

The API should feel like Pydantic for persisted data:

- Python model classes define database schemas.
- Field declarations carry type and constraint metadata.
- Validation happens at the engine boundary.
- Database-aware constraints such as `id=True` and `unique=True` are first-class.
- Returned rows feel like lightweight model snapshots.

The current project is usable as a prototype. It is not yet a production database engine.

## Current Implementation Status

Implemented:

- Python model declarations with `Model`, `Int`, and `String`.
- Field metadata: `id`, `unique`, `required`, `range`, and `default`.
- Public table API: `insert`, `find`, `update`, and `delete`.
- Returned `Row` objects with attribute access, item access, iteration, `repr`, and `to_dict()`.
- Native Rust engine loaded through PyO3 as `crustdb._native`.
- Explicit Python JSON development engine selected with `engine="json"`.
- Rust-owned persistence with `manifest.crust`, `schema.crust`, and `tables/*.tbl`.
- Stable internal `RowId` values.
- Append-only table operation logs for insert/update/delete.
- Page-based B-tree exact-match index files for `id=True` and `unique=True` fields.
- Dirty-index recovery by rebuilding indexes from table logs.
- Missing/corrupt index recovery by rebuilding indexes from table logs.
- Schema compatibility checks: incompatible persisted schemas are rejected.

Current scope and later work:

- B-tree reads use page traversal, but writes still rewrite derived index files.
- Table files are operation logs, not page-backed table storage.
- Opening the native engine still replays table logs into memory.
- Only exact-match lookup is supported.
- Multi-field filters and non-indexed filters scan live rows.
- No range query API yet.
- No non-unique secondary indexes yet.
- No WAL, transactions, compaction, or concurrent writer support yet.
- `frozen=True` exists as model metadata, but frozen rows are not enforced yet.

## Architecture

```text
Python API
  Model / Field / Table / Row
      |
      v
Python engine adapter
  |-- NativeEngine by default; requires crustdb._native
  |     |
  |     v
  |   PyO3 binding crate
  |     crates/crustdb_py
  |     |
  |     v
  |   Rust core engine
  |     crates/crustdb_core
  |     |
  |     v
  |   Native storage files
  |     manifest.crust
  |     schema.crust
  |     tables/*.tbl
  |     indexes/manifest.crustix
  |     indexes/{Model}/{field}.idx
  |
  `-- JsonEngine only when engine="json"
        |
        v
      Development JSON files
        schema.json
        {Model}.json
```

The Rust table operation log remains the source of truth. Index files are derived structures. If an index is dirty, missing, corrupt, or incompatible, the engine rebuilds it from the table logs.

## Source Tree

```text
CrustDB/
|-- .gitignore
|   # Ignores local virtual environments, pytest cache, Python bytecode,
|   # generated databases, native build artifacts, internal notes,
|   # build metadata, and Rust target output.
|
|-- README.md
|   # User-facing overview, current status, examples, setup commands,
|   # and runtime layout.
|
|-- SPEC.md
|   # This file.
|   # Current architecture/specification for the project.
|
|-- pyproject.toml
|   # Python project metadata.
|   # Configures package discovery under python/, pytest paths, uv dev deps,
|   # and maturin metadata for building crustdb._native.
|
|-- uv.lock
|   # uv lockfile for reproducible Python dependency resolution.
|
|-- Cargo.toml
|   # Rust workspace root.
|   # Includes crustdb_core and crustdb_py.
|
|-- Cargo.lock
|   # Rust dependency lockfile.
|
|-- assets/
|   |-- CrustDB_banner.png
|       # README banner image.
|
|-- examples/
|   |-- basic.py
|       # Manual smoke test for the public API.
|       # Defines a User model, opens CrustDB, registers the model,
|       # inserts or loads one row, and prints the username.
|
|-- python/
|   |-- crustdb/
|       |-- __init__.py
|       |   # Public package exports.
|       |   # Re-exports CrustDB, Model, Field, Int, String, Row,
|       |   # and custom exceptions.
|       |
|       |-- database.py
|       |   # Defines CrustDB.
|       |   # Owns the database directory path, registered models,
|       |   # table objects, engine creation, schema registration,
|       |   # and dynamic db.User-style table access.
|       |
|       |-- model.py
|       |   # Defines ModelMeta and Model.
|       |   # Collects Field declarations into __fields__, records
|       |   # __model_name__, and stores __frozen__ metadata.
|       |
|       |-- fields.py
|       |   # Defines Field, Int, and String.
|       |   # Stores Pydantic-like metadata: id, unique, required,
|       |   # range, and default. Converts fields into schema payloads.
|       |
|       |-- table.py
|       |   # Defines Table.
|       |   # Provides insert(), find(), delete(), and update().
|       |   # Delegates validation, uniqueness, storage, and lookup to the engine.
|       |
|       |-- row.py
|       |   # Defines Row.
|       |   # Wraps row dictionaries with attribute access, item access,
|       |   # iteration over field names, repr(), and to_dict().
|       |
|       |-- engine.py
|       |   # Python engine adapter.
|       |   # Uses crustdb._native.Engine by default.
|       |   # Raises if the native binding is unavailable.
|       |   # Uses JsonEngine only when selected with engine="json".
|       |   # Keeps register_schema(), insert(), find(), delete(), and update()
|       |   # behind one Python boundary.
|       |
|       |-- exceptions.py
|       |   # Defines CrustDBError, ValidationError, UniqueConstraintError,
|       |   # and reserved NotFoundError.
|       |
|       # Generated _native extension artifacts are ignored.
|       |
|
|-- crates/
|   |-- crustdb_core/
|   |   |-- Cargo.toml
|   |   |   # Rust crate metadata for the database core.
|   |   |   # Depends on serde and thiserror.
|   |   |
|   |   |-- src/
|   |       |-- lib.rs
|   |       |   # Public Rust core exports.
|   |       |   # Re-exports Engine, schema types, value types, and error types.
|   |       |
|   |       |-- engine.rs
|   |       |   # Rust engine implementation.
|   |       |   # Owns schemas, live/deleted rows, stable RowIds, validation,
|   |       |   # exact indexed lookup, insert, find, delete, update,
|   |       |   # schema persistence, table persistence, and index lifecycle.
|   |       |
|   |       |-- error.rs
|   |       |   # Rust error hierarchy.
|   |       |   # Separates validation, unique constraint, incompatible schema,
|   |       |   # unknown model, storage format, and IO storage errors.
|   |       |
|   |       |-- schema.rs
|   |       |   # Rust schema model.
|   |       |   # Defines DataType, FieldSchema, and ModelSchema.
|   |       |
|   |       |-- value.rs
|   |       |   # Rust runtime value model.
|   |       |   # Defines hashable Int, String, Bool, and Null values.
|   |       |
|   |       |-- storage/
|   |       |   |-- mod.rs
|   |       |   |   # Storage module exports.
|   |       |   |
|   |       |   |-- manifest.rs
|   |       |   |   # Creates and validates manifest.crust with storage format version.
|   |       |   |
|   |       |   |-- record.rs
|   |       |   |   # Encodes and decodes binary records.
|   |       |   |   # Provides primitive binary cursor helpers used by table,
|   |       |   |   # schema, and index storage.
|   |       |   |
|   |       |   |-- schema_file.rs
|   |       |   |   # Saves and loads schema.crust in Rust-owned binary format.
|   |       |   |
|   |       |   |-- table_file.rs
|   |       |       # Appends and replays tables/*.tbl operation logs.
|   |       |       # Supports INSERT, UPDATE, DELETE, stable RowIds,
|   |       |       # next RowId calculation, and migration from legacy row files.
|   |       |
|   |       |-- index/
|   |           |-- mod.rs
|   |           |   # Index module exports.
|   |           |
|   |           |-- manifest.rs
|   |           |   # Reads/writes indexes/manifest.crustix dirty/clean state.
|   |           |
|   |           |-- key.rs
|   |           |   # Encodes Value into stable sortable byte keys.
|   |           |   # Int keys use sign-flipped big-endian i64 ordering.
|   |           |
|   |           |-- memory.rs
|   |           |   # In-memory exact index state for id=True and unique=True fields.
|   |           |   # Used for uniqueness checks and for rewriting derived B-tree files.
|   |           |
|   |           |-- file.rs
|   |           |   # Reads/writes page-based B-tree exact-match index files.
|   |           |   # Uses CRUSTBT1, 4096-byte pages, header pages,
|   |           |   # leaf pages, internal pages, linked leaves, full-index loading,
|   |           |   # and root-to-leaf exact lookup.
|   |           |
|   |           |-- disk.rs
|   |               # Index lifecycle manager functions.
|   |               # Opens clean indexes, rebuilds dirty/missing/corrupt indexes
|   |               # from table logs, saves per-model index files,
|   |               # and exposes exact disk lookup.
|   |
|   |-- crustdb_py/
|       |-- Cargo.toml
|       |   # Rust crate metadata for the Python native module.
|       |   # Builds a PyO3 cdylib imported as crustdb._native.
|       |
|       |-- src/
|           |-- lib.rs
|               # PyO3 binding layer.
|               # Wraps crustdb_core::Engine in a Mutex.
|               # Exposes register_schema(), insert(), find(), delete(),
|               # and update() to Python using JSON payloads.
|
|-- tests/
|   |-- test_engine_boundary.py
|   |   # Tests that Table delegates behavior to the engine boundary.
|   |
|   |-- test_insert_find.py
|   |   # Tests registration, insert, find, row access, and missing-row behavior.
|   |
|   |-- test_model_meta.py
|   |   # Tests Field collection and model metadata creation.
|   |
|   |-- test_persistence.py
|   |   # Tests persistence across reopen, native storage files,
|   |   # explicit JSON storage, missing-native behavior, and native index files.
|   |
|   |-- test_unique.py
|   |   # Tests duplicate primary key and unique field errors.
|   |
|   |-- test_update_delete.py
|   |   # Tests delete/update API behavior, persistence, and unique errors.
|   |
|   |-- test_validation.py
|       # Tests required fields, wrong types, bool rejection for Int,
|       # None handling, integer ranges, and defaults.
|
|-- app.crustdb/
|   # Local generated smoke-test database output.
|   # Not source.
|
|-- internal/
|   # Local implementation planning notes.
|   # Ignored and not part of the public source tree.
|
|-- target/
|   # Rust build output.
|   # Not source.
|
|-- .venv/
|   # Local Python virtual environment.
|   # Not source.
|
|-- .pytest_cache/
    # pytest cache.
    # Not source.
```

## Runtime Data Layout

Native Rust engine:

```text
app.crustdb/
|-- manifest.crust
|   # Storage format marker/version.
|
|-- schema.crust
|   # Encoded model and field metadata.
|
|-- tables/
|   |-- User.tbl
|       # Operation log:
|       # INSERT row_id record
|       # UPDATE row_id record
|       # DELETE row_id
|
|-- indexes/
    |-- manifest.crustix
    |   # Dirty/clean state for derived index files.
    |
    |-- User/
        |-- id.idx
        |   # Page-based B-tree exact-match index for User.id.
        |
        |-- username.idx
            # Page-based B-tree exact-match index for User.username.
```

Explicit Python JSON development engine:

```text
app.crustdb/
|-- schema.json
|-- User.json
```

The JSON engine exists so the Python API can still run in targeted development and test scenarios. It is selected only with `CrustDB(path, engine="json")`; the native engine is the default, and missing native bindings raise a setup error instead of silently changing storage formats.

## Native Storage Formats

### Table Logs

`tables/{Model}.tbl` is an append-only operation log.

Supported operations:

- `INSERT row_id record`
- `UPDATE row_id record`
- `DELETE row_id`

On open, the Rust engine replays the log into memory, preserving deleted rows as tombstoned internal state and computing the next stable `RowId`.

### Index Files

`indexes/{Model}/{field}.idx` is a derived exact-match B-tree index.

Current index properties:

- File magic: `CRUSTBT1`.
- Version: `1`.
- Page size: `4096` bytes.
- Header page stores root page id, page count, entry count, model name, and field name.
- Leaf pages store `encoded key -> RowId`.
- Internal pages store separator keys and child page ids.
- Leaf pages are linked so the engine can load all persisted entries.
- Exact lookup traverses from root page to leaf page on disk.

Current write limitation:

- Insert/update/delete update in-memory index state, then rewrite the derived B-tree index file.
- Direct in-place B-tree page mutation and runtime page splitting are future work.

### Index Recovery

Before a write that changes persisted data, the engine marks indexes dirty.

Write flow:

```text
mark indexes dirty
append table operation
update in-memory rows/indexes
save derived index files
mark indexes clean
```

Open flow:

```text
load manifest.crust
load schema.crust
replay table logs
if indexes are clean:
    load index files
else:
    rebuild indexes from live rows
```

Missing or corrupt index files are treated as rebuildable because table logs are authoritative.

## Public Python API

```python
from crustdb import CrustDB, Int, Model, String


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120), default=18)


db = CrustDB("app.crustdb")
db.register(User)

row = db.User.insert(id=1, username="alice")
found = db.User.find(id=1)
updated = db.User.update(where={"id": 1}, values={"age": 26})
deleted = db.User.delete(id=1)
```

Semantics:

- `insert(...)` returns a `Row`.
- `find(...)` returns a `Row | None`.
- `update(where={...}, values={...})` returns the updated `Row | None`.
- `delete(...)` returns `bool`.
- Empty `find`, `delete`, and `update` filters are rejected.
- Unknown fields are rejected.
- Indexed single-field filters use disk B-tree lookup in the native engine.
- Other filters scan live rows.
- `CrustDB(path)` uses the native Rust engine and raises if `crustdb._native` is unavailable.
- `CrustDB(path, engine="json")` explicitly uses the development JSON engine.

## Pydantic-Inspired Characteristics

Implemented characteristics:

- Model classes define schemas declaratively.
- Field declarations carry type and constraint metadata.
- `id=True` and `unique=True` are database-aware constraints, not just object validation.
- Required fields, defaults, and integer ranges are validated at the engine boundary.
- Returned rows behave like lightweight model snapshots through attribute access, item access, and `to_dict()`.
- Persisted schemas are treated as database contracts; incompatible schemas are rejected.

Desired characteristics:

- `frozen=True` should be model-level, not row-level.
- A frozen model should return read-only rows.
- Row mutation should not be the update path; database changes should go through explicit `update(...)`.
- Custom validators should be possible later, but should compile into clear engine-side validation rules where possible.
- Schema migrations should be explicit rather than silently changing persisted data.
- The future Rust API should mirror the model-first idea with `#[derive(CrustModel)]` and field attributes such as `#[crustdb(id)]`, `#[crustdb(unique)]`, and `#[crustdb(frozen)]`.

Desired frozen behavior:

```python
class User(Model, frozen=True):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


user = db.User.find(id=1)
user.age = 26  # should raise ValidationError or FrozenRowError

db.User.update(where={"id": 1}, values={"age": 26})  # correct
```

## Rust API Status

Rust usage is possible at the low-level `crustdb_core` layer, but there is not yet a polished Rust user API.

Currently exposed from `crustdb_core`:

- `Engine`
- `ModelSchema`
- `FieldSchema`
- `DataType`
- `Value`
- `CrustDbError`
- `Result`

The current Rust API requires manual schema and record construction. Desired future Rust API:

```rust
#[derive(CrustModel)]
#[crustdb(frozen)]
struct User {
    #[crustdb(id)]
    id: i64,

    #[crustdb(unique)]
    username: String,

    #[crustdb(range = "0..=120")]
    age: i64,
}
```

This would likely require a separate derive-macro crate and a higher-level Rust-facing `CrustDB`/`Table<T>` API.

## Error Model

Python exceptions:

- `CrustDBError`
- `ValidationError`
- `UniqueConstraintError`
- `NotFoundError` reserved for future APIs

Rust errors:

- `Validation`
- `UniqueConstraint`
- `IncompatibleSchema`
- `UnknownModel`
- `StorageFormat`
- `Storage`

The PyO3 binding maps validation-like Rust errors to `ValueError`, which the Python adapter translates into CrustDB-specific Python exceptions where possible.

## Verification Commands

```powershell
uv sync
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
uv run --with maturin maturin develop
uv run pytest
cargo test
uv run python examples/basic.py
```

Current expected verification at the last full run:

- `cargo test`: 41 passed
- `uv run pytest`: 28 passed
- `uv run python examples/basic.py`: prints `alice`

## Next Technical Phases

Recommended next phase:

```text
Phase 7: incremental B-tree writes
```

Phase 7 should add:

- an index manager abstraction
- direct B-tree page insert
- direct B-tree page delete
- leaf splits
- internal page splits
- new-root creation
- continued dirty-index recovery
- no full index-file rewrite after every write

Later phases:

- page-backed table storage
- table compaction
- WAL and transaction recovery
- range scans
- non-unique secondary indexes
- schema migrations
- custom validators
- enforced frozen rows
- polished Rust derive-style API
