# Hagency Hosting for macOS

Hagency Hosting controls a separate owner daemon. The same dashboard appears in
Hagency's Agent page. Closing either dashboard does not stop the daemon.

## Build and enable

From the Desktop repository, with the Client repository alongside it:

```sh
python3 packaging/owner-controller/build.py --release --client ../hagency-client
```

The output is `.run/owner-daemon/dist/Hagency Hosting.app`. The build includes
the native accessibility adapter and daemon helper, copies resources and
licenses, and verifies an ad-hoc signature. It does not install a service or
change an account. A distributable release still needs its normal signing and
notarization pipeline.

For an existing Hagency owner, open background-hosting setup from that owner's
Agent page. This passes the existing resource store to the controller. Review
the owner and server, then select **Enable hosting**. The transfer briefly
pauses new work, preserves Agent identities and stored data, and fences the
previous executor. Browser sign-in authorizes the daemon's own Matrix device.
Interrupted enrollment can be retried with the saved device.

For a new owner, launch Hagency Hosting directly and enter the Matrix server.
The owner account field is optional; browser sign-in determines the account.
Enabling hosting installs the per-user `io.hagency.owner-daemon` LaunchAgent.
It runs while that macOS user is signed in. The machine must remain awake and
connected for remote requests to execute.

An existing development Desktop launched with
`--hagency-isolated-installation-root` needs a matching development helper built
with `shared-budget-execution,test-support`. Pass that same absolute installation
root when running `hagency --hagency-isolated-installation-root PATH daemon install`.
The generated service preserves the root across restarts. The ordinary release
helper rejects this development option; its canonical registry selection is
unchanged. The installation root and `daemon enroll --adopt-root` are different:
the former selects the existing development registry, while the latter selects
the existing native owner store. Neither requires moving or rewriting the ledger.

Use the owner menu to move the dashboard between Hagency and its standalone
window. The destination must acknowledge the matching owner's restored view
before the source closes. A failed handover leaves an actionable error in the
source. Reviewing approvals opens the corresponding Hagency page and keeps
the controller open.

## What the dashboard measures

The attached dashboard reads snapshots every two seconds. Supported provider
reports refresh independently in the daemon every five minutes and on manual
refresh. Each report retains its observation time; these are not provider
push subscriptions.

| Source | Available evidence | Boundary |
| --- | --- | --- |
| Codex | Account/plan metadata, reported quota windows and resets, runtime token/cache observations | Window sizes and available fields come from the provider. A missing absolute allowance is unknown. |
| Claude | Personal Agent runtime token/cache observations | Complete idle subscription quota refresh is not implemented. Missing plan data is shown as unavailable. |
| Octos | Personal Agent runtime token/cache observations | No universal upstream coding-plan limit exists; missing upstream data is unknown. |
| Resource pools | Measured rental usage, resource attribution and owner-set budgets | Managed rental execution currently supports Codex. Metering support does not enable Claude/Octos pooled execution. |

Resources sharing a verified provider account share its plan report. Provider
account usage may include activity outside Hagency; it is distinct from the
locally attributed resource usage. Missing cache evidence, partial reports and
older records remain unknown, not zero. The live Codex adapter applies the
documented zero default for an omitted optional cache-write counter; explicit
invalid values and missing required counters remain incomplete. No coding-plan
allowance is hardcoded.
Agent budgets remain owner controlled; this dashboard adds no Room budgets.

Usage can be filtered by account, resource, model, requester and date range.
Long filters use name/ID search and pagination. Refresh preserves drafts,
selection and reading position. Different chat rooms use separate, stable
working directories without granting additional file permissions.

Coverage begins when collection starts. The first day, week or month can be
marked partial while showing correctly measured usage; earlier history remains
unknown. Restarting the daemon preserves the recorded usage and coverage start.

## Validation

See [ADR 0012](../../docs/adr/0012-owner-daemon-shared-dashboard.md) for the
architecture and acceptance requirements. The `tools/owner-dashboard-*`
scripts exercise isolated fixtures; `tools/owner-controller-bundle-smoke.py`
tests the signed bundle's LaunchServices identity and daemon independence.
The Client's `owner_daemon_crypto` and explicitly opted-in
`pooled_daemon_crypto` integration tests exercise real Matrix encryption,
controller detachment and process restart with deterministic providers. The
pooled test additionally covers canonical adoption, allocation and settlement.
None of these tests requires migrating a live user's profile.
