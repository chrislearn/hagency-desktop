# ADR 0012 implementation and acceptance record

Date: 2026-11-01. Status: implementation, automated runtime validation, numerical
UX review and final builds completed. Manual VoiceOver acceptance remains open.
This is not a release approval.

Amended 2026-11-05: Client ADR-192, "One owner identity per computer"
(hagency-client `knowledge/decisions/adr-192-one-owner-identity-per-computer.md`),
folds Hagency Hosting into Desktop and removes the standalone app, dashboard
handover and setup launch, with their bundle, guide and smoke tests. Evidence
below for those parts is historical. The source-pin and regression gates now
cover two workspaces: Desktop and the owner-dashboard crate.

The implementation spans the Hagency Client and Desktop work branches. The
Server protocol was exercised against a disposable local deployment. No live
owner was migrated, no LaunchAgent was installed on the user's account, and no
production deployment or commit/push was performed for this validation.

## Delivered implementation

- Independent owner daemon, private local IPC with OS-user and enrolled-owner checks,
  durable command receipts, explicit custody transfer and restart recovery.
- One native dashboard component in the standalone macOS controller and Hagency
  Agent page, with shared commands, views, localization and theme behavior.
- Usage ledger with personal/rental attribution, token/cache categories,
  filtering and unknown/partial/stale states. Codex provider reports refresh
  without an inference request. Provider coverage and limits are documented in
  [ADR 0012](0012-owner-daemon-shared-dashboard.md#provider-coverage).
- Explicit same-owner view handover and acknowledgment before the source closes.
  Approval navigation opens the corresponding tool/resource-request review page.
- Stable per-room personal Agent directories, preserving file permissions and
  existing data. Modern Matrix event IDs are accepted for encrypted confirmation.

## Verified runtime and regression evidence

| Check | Result | Scope |
| --- | --- | --- |
| Mandatory Desktop/Client regression driver | 64 checks passed; 342 Rust tests | Full initial run; all 23 native checks rerun after the quota and metering fixes, with 41 unaffected Desktop checks retained |
| Client daemon suite | 73 tests passed | Includes bare-origin SDK issuer restoration and bounded RPC dispatch futures; rerun after quota and metering fixes |
| Client default and shared execution builds | Both final checks passed | Default and `shared-budget-execution` configurations, rerun after the final async-phase fix |
| Enrolled daemon lifecycle | Passed | Two controllers detach; independent SDK session remains usable and survives restart without a second enrollment |
| Encrypted personal Agent daemon subprocess integration | Passed | Real Matrix encrypted DM and mentioned three-member group; unmentioned group ignored; restart preserves keys/context; distinct room directories; three turns without replay |
| Adopted pooled Agent daemon integration | Passed | Real adoption, publication, allocation and idle quota reads; two encrypted replies with both controllers detached, daemon restart, stable crypto/cwd, exactly two model turns and ten consumed tokens |
| Legacy adopted-resource quota | 1 adoption gate and 11 identity tests passed | Original authenticated provider home, canonical account/revision checks, namespace handling, account changes and retirement during observation |
| Codex optional usage fields | 36 focused tests passed | Installed 0.166.0 schema, live decoder, generic history, dashboard and conservative settlement; missing required fields remain incomplete |
| Encrypted receipt recovery | 3 tests passed | Structured private records larger than 512 bytes survive replay; read/write bounds, oversized rejection and file privacy remain enforced |
| Deferred shared execution phases | 3 focused tests passed in both test configurations | Cancellation, deadline and bounded construction; actual pooled restart runs on default 2 MiB Tokio workers without a stack-size override |
| Final encrypted execution regression | 2 tests passed | Missing keys cannot start inference; original and successor execution retain plaintext locally, rerun on the frozen compiled library |
| Pooled Server usage aggregation | 1 isolated PostgreSQL regression passed, plus full pooled integration | Includes agents without Project allocations while retaining scope isolation, settlement deduplication and overflow refusal |
| Final packaged macOS lifecycle, v8 | 7 checks passed | Ad-hoc signature, separate regular Dock identity, close/reopen/quit and independent daemon survival on the final helper |

The encrypted integration uses a deterministic provider and disposable accounts,
server and databases. It proves the actual daemon host's encryption and execution
path in a separate process; it does not prove paid inference, every provider or
the packaged LaunchAgent's complete enrollment journey. The bundle lifecycle
test uses an unenrolled daemon and does not claim encrypted Agent execution.

The final pooled test verifies two complete five-token work receipts and a Server
known-consumption total of ten. Its active owner-set ceiling and conservative
commitment remain 1,000 tokens; spending ten does not release an active Agent's
reserved capacity. The restored dashboard reports six input and four output
tokens, two executions, no unmeasured executions and no capture errors. Its
calendar period is correctly marked partial because it begins before collection
started. Historical coverage and completed-turn accounting are separate evidence.

The runtime checks exposed and fixed oversized async poll frames, a 512-byte
secret-file limit incorrectly applied to structured encrypted-delivery records,
and a Server join that omitted pooled agents without Project allocations.
Execution fences, receipt ordering and owner-defined budgets remain enforced.

Local detailed logs are retained under `.run/owner-daemon/`:
`regressions-final/report.json`, `regressions-integrated/report.json`,
`final-client-integrated-report.json`,
`review/enrolled-daemon-lifecycle.log`, `review/daemon-crypto-e2e.log`, and
`review/final-code-review.md`. Quota and metering evidence is in
`review/quota-fix-final-report.json` and `review/metering-contract-review.md`.
Final recovery evidence includes `review/pooled-daemon-e2e.log`,
`review/pooled-restored-dashboard.json`, `final-client-phase-checks.json`,
`final-native-encrypted-phase.log`, and the independent receipt, shared-phase
and pooled-usage reviews under `review/`.

## Final local builds

The standalone bundle is
`.run/owner-daemon/dist-final-v8/Hagency Hosting.app`. The integrated Desktop
release executable is
`.run/owner-daemon/targets/accessibility-embedded-release/release/rinx`.
Both builds include the frozen Client fixes; no fixture feature is enabled in
these production builds. The bundle is signed ad hoc for local review.

| Artifact | SHA-256 |
| --- | --- |
| Standalone controller | `2847152cbc2999cdb5162ec0bb84fb0b8f15177ee45a3a7e295a3338b5af2ccb` |
| Bundled daemon helper | `3276f1378e1a80e7dc3aef73e618a8338d11669eaef318bc71e3b766e3c97a3e` |
| Integrated Desktop executable | `b5ce9b1fa7522d4de78f884dd5cbba960e158e6fa1236688560b94f0df180e81` |

`.run/owner-daemon/final-builds.json` and `final-source-manifest.json` record
artifact provenance and verified source hashes. Default Git-source lockfiles
were restored after the native framework builds; all three Desktop/controller/
dashboard workspaces pass offline locked dependency resolution. The final test
harness exited successfully and removed its disposable processes, databases and
profiles. The existing PostgreSQL container was left running.

## UX acceptance

Independent Codex review scored the standalone dashboard **95.7/100** and the
embedded dashboard **95.5/100**, with every dimension at least 94. Both exceed
the strict numerical target. These are observed expert assessments, not user
study measurements or a complete release signoff. The reviewer found no blocking
UI defect in the inspected final journeys. Minor deductions include dropdown
label alignment and the scrolling needed for exceptionally long names.

| Final instrumented candidate | SHA-256 |
| --- | --- |
| Standalone | `cf9d1cb0e7e99e262e6df4047f8197ffb3977e35fd6107e607f94bc229d4436c` |
| Embedded | `0d6fbcd043f519bb1a1eb31d90c29cfe761584e8b753836570ecd79f076cc0c7` |

Final evidence includes 87 shared component tests, 41 standalone and 24 embedded
native checks, eight standalone and six embedded visual/retention matrices with
10 checks each, and exact-candidate English/Chinese primary journeys with 12
checks each. Two UI processes passed 13 handover checks: the source stays open
until the receiving UI restores and acknowledges its view; the reverse direction
and simultaneous independent controllers also pass. That test uses synthetic
data and prestarted peers; LaunchServices was tested separately.

Both hosts retain drafts, focus, selected text and geometry across ten refreshes.
The reported local input acknowledgment p95 stays below 5 ms on the recorded
machine and fixture load. Actual host typography grows populated glyphs by
1.519× in the standalone host and 1.515× in the embedded host when set to 150%.
These measurements distinguish host typography from whole-window zoom.

The full local independent review and deductions are recorded in
`.run/owner-daemon/review/final-ux-review.md`. Embedded-host reproduction
scripts are listed in the [native accessibility guide](../../packaging/owner-accessibility/README.md).

Headless tests cover actual control actions, geometry, selection, draft/focus
retention and input acknowledgment. Native tests exercise AppKit accessibility
actions and focus, text composition/commit and reduced-motion rendering.
These tests do not constitute a user-operated VoiceOver reading/navigation or
spoken-announcement session. That release acceptance item remains open.

The measured embedded production panes passed at 420×640 with 200% text,
768×720 and 1200×800. The production dock minimum is 420 logical points. A separate
360×640 component fixture passed at 200%; it does not claim that the production
splitter can reach 360. Standalone windows were tested at 400×540, 520×720 and
1080×800, with English/Chinese, light/dark and 100/125/150/200% text variants.

## Review corrections, 2026-11-02

- The command model distinguishes a local failure to submit from a daemon
  rejection. A failed check keeps the original uncertain ticket, including its
  revision, even when snapshots subsequently refresh. Both hosts use this rule.
- Hosting mutations reconnect and verify the enrolled owner before sending, so
  background reads cannot leave the next command on an exhausted connection.
- Handover cleans validated expired tickets across accounts, removes the sender's
  outgoing ticket on account switch, and expires unanswered UI restorations.
- Setup launches a separate controller instance and waits for the exact form to
  acknowledge its owner/server/root. LaunchServices acceptance alone is not
  treated as setup success. This creates no second executor or enrollment.
- The standard release gate now tests the dashboard and controller crates and
  builds the controller entry point. All three workspaces must pin the same
  published Client revision, including their lockfiles.
- Recurring budget facts stay visible independently of translated labels.
  Repeated renewal warnings are limited to once per ten minutes per error,
  with immediate logging for changed errors and after recovery.
- Idle hidden dashboards release their connector; uncertain/in-flight commands
  retain their receiver. The unused scope-changing job variant was removed.

The dashboard IPC uses same-OS-user credentials and a single enrolled owner,
not per-controller revocable leases. The amended ADR distinguishes it from the
native management bridge's controller leases. Native accessibility remains an
explicit special build using the tracked exact-pin Makepad patch described in
`packaging/owner-accessibility/README.md`; a normal build does not
claim native accessibility acceptance. Neither a standard-build test pass nor
a rendered screenshot closes the manual VoiceOver acceptance item.

Validation artifacts for this review are retained under
`.run/adr0012-followups/`. Source publication order is Server, Client, then Desktop
with all Client dependencies and lockfiles repinned. This review does not claim
that Astra's historical incomplete usage has been settled or that its new
capacity has been published.

The review's Server changes are published on
`work/hagency-server-local-20261028` at `2f4a2cdd5665ddbb13add394a7976dd2805e9558`.
Client is published on `work/hagency-client-local-20261028` at
`fcbd247b17bae376e9dec98895e43980bc16d193`; Desktop and both owner UI workspaces
resolve this exact Git revision. The source-pin gate checks all three lockfiles.

The final follow-up evidence includes 452 passing PostgreSQL service tests,
78 daemon tests, 82 metering tests, and 11 recurring-period tests (including the
two runtime fixtures explicitly enabled with the freshly built offline probe).
The metering suite's two optional integration cases remain ignored. The real
two-process handover journey passes 14 checks, including a destination initially
outside the Hosting page. The setup launch test reproduces macOS ignoring
arguments on ordinary reopen and passes six checks with a separate setup
instance and an exact-form acknowledgment. It uses an isolated bundle rather
than the installed production application's bundle lookup. Native AppKit
accessibility passes 41 automated checks on the patched framework build.

After repinning, the complete Desktop regression driver passes all 77 checks,
including 497 Rust tests and the standalone production controller build. The
embedded instrumented executable is also rebuilt against the published Client
revision before the two-process handover check. The passing gate report is
`.run/adr0012-followups/pinned-regressions/report.json`.
