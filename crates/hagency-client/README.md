# Hagency native libraries

Desktop owns this source tree. Its `hagency`, `hagency-core` and owner-dashboard
metering dependencies use local paths; no separate Client checkout, Git pin,
Node build or browser console is needed. The 18-crate Cargo workspace remains
independent so its backend tests and lockfile can run without building the GUI.

`native/hagency` is the embedding library. Desktop dispatches its guardian and
`daemon` command before starting the GUI (`src/main.rs` at the repository root).
The library retains owner state custody, private daemon IPC, Matrix encryption,
Agent execution, history, questions, approvals, accounting and management APIs.
The historical `console` Rust module name denotes those backend APIs, not a UI.

The old `mockup` frontend, static asset server, asset embedding build script,
standalone UI executable and its `start` / `open` / UI service installer are
retired. Server OAuth enrollment may still open the server's login page. No
existing user state is migrated, unlocked, deleted or reclassified by this
source move; owner markers, database schemas and protocol identifiers remain
compatible.

From the Desktop root:

```sh
cargo test --locked --manifest-path crates/hagency-client/Cargo.toml --workspace \
  -- --skip native_codex_real_app_server \
  --skip native_two_agent_qualification_records_its_evidence
cargo test --locked --manifest-path crates/hagency-client/Cargo.toml -p hagency \
  --features shared-budget-execution,resource-adoption --lib
python3 tools/check-hagency-regressions.py --output .run/regressions
```

Historical qualification tests require explicit external credentials or runtime
setup. Most are ignored by default; the two named exclusions above require a live
qualified host and match CI. The [library CI job](../../.github/workflows/main.yml)
shows the offline runtime peer setup for a full suite. The regression runner
builds its own offline probe for the encrypted question/history execution checks.
The Linux containment qualification helper and its ordinary-file refusal tests
remain under `native/scripts`; CI runs the refusal tests without provisioning
cgroups. Actual containment qualification requires a disposable Linux runner.
The explicit binaries under `tests/fixtures` are subprocess test peers, not
packaged applications. `hagency-owner-test-host` exercises state locks, recovery
and headless HTTP APIs without restoring the old UI.

`UPSTREAM.json` records the original import, not a live dependency revision.
After import, the Desktop commit identifies both application and library source.
The original Apache-2.0 `LICENSE` and `NOTICE` are retained here.
