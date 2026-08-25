# Repository Guidelines

## Architecture Source of Truth

`../tasktips/docs/tasktips-server-sync-design.md` is the first-version product and architecture baseline. Keep this repository compatible with the desktop app's `SyncObject`, `SyncTombstone`, revision, raw-byte SHA-256, and tombstone semantics. A contract or behavior change must update the design document and `contracts/openapi.yaml` in the same change.

The service is privacy-sensitive. A `system_admin` may manage accounts, inspect operational metadata, and request restores, but must never receive Todo text, classification/index payloads, images, RustFS object keys, download URLs, or storage credentials.

## Project Structure and Boundaries

- `crates/domain`: framework-independent types and rules. It must not depend on Axum, SQLx, or AWS/RustFS SDKs.
- `crates/application`: use cases and ports over the domain.
- `crates/api`: Axum routes, Tower middleware, authentication, validation, and HTTP error mapping.
- `crates/persistence`: PostgreSQL, SQLx transactions, migrations, and RLS integration.
- `crates/object-store`: RustFS S3 operations, hashing, streaming, and immutable object layout.
- `crates/worker`: durable jobs for snapshots, restores, purge, cleanup, and statistics.
- `admin`: Vue 3 management application. DTOs come from `contracts/openapi.yaml`; do not hand-write duplicate API models.
- `tests/{contract,integration,e2e}`: cross-crate tests; keep focused unit tests beside Rust or TypeScript modules.

PostgreSQL owns identity, authorization, revisions, heads, change logs, idempotency, jobs, and audit records. RustFS owns immutable payload bytes only. Write and verify a payload before committing its database reference. Never delete an object-store payload while a database reference still exists.

## Commands and Validation

Use the pinned Rust toolchain and committed lockfiles.

- `cargo fmt --all -- --check`: check Rust formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint all Rust targets.
- `cargo test --workspace`: run Rust tests.
- `cd admin && npm ci`: install the exact frontend dependency graph.
- `cd admin && npm run check`: lint, test, generate/check the API client, and build.
- `docker compose --env-file deploy/.env -f deploy/compose.yaml config`: validate deployment configuration.
- `docker compose -f deploy/compose.dev.yaml config`: validate the development stack (PostgreSQL and RustFS only, localhost-bound, fixed dev credentials).

Run the smallest relevant checks while iterating, then run every directly affected check before handoff. Docker-dependent PostgreSQL, RustFS, image, and Compose tests require a working Docker engine; report them as unverified when Docker is unavailable.

## Style and Tests

Use rustfmt defaults. Rust modules/functions use `snake_case`; types and traits use `PascalCase`. Prefer typed errors and structured tracing fields. JSON fields use `camelCase`. Never log payload bodies, tokens, passwords, RustFS credentials, signed URLs, or internal paths.

Use two spaces in TypeScript, Vue, JSON, and YAML. Vue components use `PascalCase.vue`; TypeScript values use `camelCase`; other filenames use kebab-case. All visible admin text must use vue-i18n keys, defaulting to `zh-CN`. Add regression tests for fixes. Cover CAS, generation/cursor validation, idempotency, RLS isolation, hash/size checks, token rotation, restore behavior, and stable error-to-status mappings as those features land.

## Contract, Migration, and Security Rules

`contracts/openapi.yaml` is the only HTTP contract source. Treat removed fields, changed enum meaning, tighter published constraints, and incompatible status/error changes as breaking. Use a new `/api/vN` path for breaking APIs. Generated clients must be reproducible and committed.

Database changes require forward-only SQLx migrations and tests from an empty database and the previous supported schema. Production business roles must be non-owners with forced RLS. Keep `/metrics`, PostgreSQL, and RustFS internal. Public traffic enters through Caddy on one origin; do not add broad CORS.

## Git and Review

Use Conventional Commits: `<type>(<scope>): <imperative summary>`. Allowed types are `feat`, `fix`, `docs`, `refactor`, `test`, `build`, and `chore`. Keep the subject lowercase, omit the final period, and limit it to 72 characters. Mark breaking changes with `!` and a `BREAKING CHANGE:` footer. Keep commits focused and do not stage unrelated user work.

Reviews must state behavior, verification, schema/contract impact, deployment impact, and remaining risk. UI changes include light/dark screenshots at common desktop widths. Security-sensitive changes explicitly verify that admin responses and logs contain no user payload.

