# Hagency Client library integration

Desktop now owns the native backend source under `crates/hagency-client`.
The import starts from Client commit `6cd99ceb84ae407b2315dcd0f5e900022eebac06`,
the exact revision shipped in Desktop 0.5.0-rc.4. `UPSTREAM.json`, LICENSE and
NOTICE retain that provenance. Subsequent changes belong to the Desktop commit.

Desktop and owner-dashboard use local path dependencies. The native 18-crate
workspace keeps its own manifest and lockfile so backend tests can run without
the GUI. The Desktop and dashboard lockfiles change only the native packages'
source from Git to local paths; third-party versions are preserved.

The removed product surface consists of the `mockup` web frontend, the static
asset loader/server, `HAGENCY_CONSOLE_DIR` build-time embedding, the standalone
`hagency` UI executable, its browser access socket, and the old UI service
installer. Desktop remains the UI and dispatches the library's existing daemon
and guardian entry points. The daemon's server OAuth enrollment and private
management IPC remain available.

The Rust `console` module remains because it also owns runtime, encryption,
accounting, authentication and management code used by Desktop. Its headless
constructors no longer require assets. API paths and state formats are retained;
this refactor does not perform a data migration or alter budget settlement.
The retired socket filename is accepted in an existing owner directory for
compatibility, but no listener or access link is created for it.

Backend integration tests now construct headless API state. Process/restart
fixtures use `hagency-owner-test-host`, a test-only source executable, instead
of the removed product UI CLI. Browser-only asset, page-rendering, CLI and
installer cases are retired; API authentication, state/refusal/restart tests
remain, with new assertions that old UI routes return 404. Desktop's existing
chat, approval, question and hosting regression gates remain in place.

The regression runner always tests the in-tree library. CI includes its full
workspace tests and shipping-feature library tests, and release CI no longer
checks out Client separately. Release manifests record `clientPath` plus the
same commit for Desktop and its library. Source checks reject external native
dependencies, stale Git lock entries and a restored standalone browser build.
