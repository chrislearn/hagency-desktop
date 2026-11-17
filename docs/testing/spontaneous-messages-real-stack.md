# Real Desktop / Server / Palpo acceptance

The 2026-11-17 run used Desktop product revision `f6a1ad00`, Server
`6617981d`, Palpo `60c25e6516fd72e8e64f9d7631ac060a35afd612`, and Pasion
`37d6e30ceecb9c0aee3da3c6990298c13760f4bc`. The native Desktop and Server
executables were built locally from these revisions. All accounts, rooms,
database names, profiles, and daemon homes were disposable.

This verifies personal Agents in encrypted DMs and multi-user encrypted rooms.
The model boundary is a deterministic Codex protocol subprocess, with no
inference or paid account. The Matrix transport, OAuth, application service,
database, daemon, process guardian, SDK encryption, and native UI are real.
It does not establish completion of ADR 0013 or enable the disabled native
Octos report experiment.

## JIT rerun and readiness correction

The first two JIT attempts timed out on the first encrypted DM with no model
turns. Subsequent investigation verified the Agent's device signature with an
independent Ed25519 implementation. SDK debug logs then showed successful Olm
key delivery and Megolm room-key import. Three full JIT daemon runs passed, with
fresh device grants, once with SDK debug logging and twice at the normal level.
Each delivered 10 distinct reports and 5 final answers through real Palpo,
including two humans, encrypted groups, and process restart. No native build
was used for these reruns.

The old acceptance sender used a standalone HTTP `/keys/query` as its only
readiness check. That observes the server but does not refresh the sender's
SDK crypto cache. The production Desktop send path already guarded this case.
Its signature-checked device-discovery helper now lives in
`hagency::agent_encryption::wait_for_sendable_device`, and both Desktop and the
source integration fixture use it. The helper has a 30-second bound and
preserves device trust, encryption, and history-sharing policy.

All three readiness tests passed under JIT: discover keys after an empty
response, reject invalid signatures, and preserve blacklist policy. The
runner logged a nonfatal `ruma_macros` documentation-body compiler diagnostic;
the selected tests completed with exit status 0. Full daemon reruns used the
existing precompiled orchestration driver, so they exercised its older HTTP
readiness check; they do not establish that the initial transient failure is
fully explained by the fixture difference. The changed shared helper was
executed by the JIT tests.

A hidden Desktop JIT also completed real Pasion DCR/PKCE and reached its signed-in
home. Its device keys were published before testing offline delivery. After offline delivery and daemon restart, the same UI profile restored
and rendered six independently actionable Agent rows: four equal-text reports
and two final answers. The selected room, semantic actions, and actual GPU
frame were checked. A separate SDK client drove the input requests; the model
boundary remained a deterministic protocol subprocess. Existing user JIT processes were preserved; each run used an idle
host, private state, and existing native dependency/helper artifacts. No
Element verification was performed.

## Historical native acceptance (before the JIT-only instruction)

- Embedded Pasion and Palpo startup and the mandatory durable Appservice event
  roundtrip, without a Matrix mock.
- `real_palpo_delivery_only_report_authorization_and_revocation`: the real
  Desktop v2 encrypted outbox calls Server's delivery-only authorization API
  and delivers two distinct equal-text reports to a real SDK receiver. A
  duplicate native ID sends once, reopening retains receipts, and owner
  leave/rejoin permanently fences the old binding generation. No model runs.
- `real_daemon_encrypted_dm_group_restart`: the actual Cargo-built Desktop
  executable runs `daemon run`, including its production guardian entry point.
  Two UI controllers detach; the daemon continues five provider turns across
  encrypted DM/group requests and a process restart. Ten independent reports
  and five final answers arrive. Duplicate provider items do not add events.
- Two separate real human Matrix accounts decrypt the same six group reports.
  Group reports retain the exact request thread. Unmentioned group messages
  do not execute a model. The Agent's crypto identity survives restart. All
  fifteen output events are `m.room.encrypted`, without a plaintext body on
  the wire.
- A separate hidden native Desktop signs in through real Pasion DCR/PKCE and
  the SDK callback. Its rendered timeline shows both v2 reports separately,
  with individual actions and an Agent badge. A second capture shows six
  separate daemon messages across the DM restart: four reports and two final
  answers. Semantic rows and actual GPU frames were both inspected.

The fixture's owner explicitly publishes its test Agent through the supported
Server API before binding the multi-user room. Desktop's native creation
contract currently creates a private Agent; this test does not silently change
that default or claim to test a visibility-setting UI.

## Development acceptance: JIT only

Run Desktop UI and its actual owner daemon under the pinned Makepad JIT.
Reuse existing matching runner, native dependency cache, guardian helper, and
Server binary. Native compilation is reserved for release work. If a matching
artifact is unavailable, report that dependency; do not silently run a Cargo
build. A pre-existing test driver may orchestrate the JIT product.

Use an idle host, one JIT at a time, with a memory cap, a private profile, and a
separate daemon home. Preserve existing user processes and never use hosting
Setup. Use `umask 077` for disposable fixture files and directories.

The Server's `scripts/test-agent-integration.py` must run with
`HAGENCY_TEST_HOLD_CLIENT=1`. Its normal startup and Appservice checks remain
mandatory. `HAGENCY_TEST_NATIVE_ONLY=1` omits browser asset assertions; genuine
Pasion resources, including public assets, are still required. Enable fixture
management capabilities with `HAGENCY_TEST_MANAGEMENT=1` when testing that UI;
record whether this optional capability was enabled.
`HAGENCY_TEST_CLIENT_TIMEOUT_SECONDS` extends the bounded deadline. A private
`HAGENCY_TEST_POSTGRES_FIXTURE` descriptor selects disposable native loopback
PostgreSQL instead of Docker; see the Server script docstring.

After the Server announces its private `client-fixture.json`, prepare grants:

```sh
umask 077
python3 crates/hagency-client/native/hagency/tests/fixtures/prepare_daemon_crypto.py "$CLIENT_FIXTURE"
```

The daemon test driver accepts an absolute executable launcher through
`HAGENCY_DAEMON_CRYPTO_BINARY`. The launcher forwards `daemon run --home ...`
to this JIT command and provides `HAGENCY_NATIVE_HELPER` as an existing native
Desktop binary for guardian subprocesses:

```sh
.sources/makepad/target/release/makepad run --no-live --allow-c rinx --test owner_daemon_jit \
  --features palpo-instrument,development-credential-files,development-native-helper,onboarding-test-support \
  -- daemon run --home "$ISOLATED_DAEMON_HOME"
```

The `owner_daemon_jit` entry is restricted to test-support builds. It selects
only disposable writer custody, preserving the production daemon CLI, owner
root, crypto store layout, and SDK code. Installed OS-owner resources remain
untouched. Native dependencies must already be prepared; prevent a cold cache
from invoking native compilation during this development run.

Run the existing `owner_daemon_crypto` driver with
`real_daemon_encrypted_dm_group_restart --ignored --test-threads=1 --nocapture`.
Set `HAGENCY_DAEMON_CRYPTO_FIXTURE` to the private prepared grants. For subsequent
UI checks, set `HAGENCY_DAEMON_CRYPTO_DEFER_DONE=1`: the successful driver writes
`daemon-crypto-result.json`; the UI cooperator writes private `client-done.json`
only after its own assertions pass. Start the UI JIT after the daemon exits.

Use fresh grants for independent cases and refresh short-lived OAuth tokens
through Pasion. Never initialize two crypto stores for the same Matrix device.
Receiver stores are scoped by role/device inside the private fixture directory.
Respect login rate limits. Keep credentials and screenshots out of commits.

## Evidence and limits

Local evidence is under `.run/spontaneous-e2e/`: `acceptance-result.json`,
`delivery-only.log`, `daemon-crypto.log`, Server harness output, semantic UI
snapshots, `ui/delivery-only.png`, and `ui/daemon-dm.png`. The acceptance manifest
records the exact binary hashes. Credentials remain in private disposable
fixture files and are not committed.

The run does not test Element, pooled execution authorization, or a browser
frontend. A genuine prebuilt Pasion public bundle was retained for startup
asset discovery; its JavaScript was not executed. The optional management
capabilities were disabled in this fixture, so Desktop's shared-permission
notice displayed a retry warning. Message rendering passed; the pooled
permission UI is not included in that result.
