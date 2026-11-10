# Native macOS accessibility for the owner dashboard

The shared `owner_dashboard::accessibility` module composes dashboard and host
controls into one tree per native window. Desktop calls `after_event` once,
after the complete UI handles the event, and registers a `ControlAdapter` for
its navigation rail, including Agent.

Native accessibility is an explicit patched-framework build: Desktop's optional
`owner-accessibility` Cargo feature enables
`hagency-owner-dashboard/native-accessibility`, which needs the tracked
`makepad-accessibility.patch` in this directory. Ordinary fixed-Git Makepad
builds do not include it. A build without the feature has an explicit no-op
semantic publisher and must not be presented as native accessibility verified.

## Source preparation

```sh
python3 tools/prepare-owner-accessibility.py --output-config .run/owner-daemon/native-accessibility.cargo.toml
```

This creates an isolated Makepad checkout at the script's `PIN` (Desktop's
Makepad revision), applies the patch, and writes a Cargo configuration with
absolute paths. The directory name includes the patch SHA-256. Reuse verifies
HEAD and all changed file hashes and rejects unexpected edits. User Makepad
checkouts and Cargo git caches are read only. Desktop's historical Makepad
source aliases are patched directly because Cargo patches are not transitive.
Path patches re-resolve `Cargo.lock`, so build from a disposable checkout or
worktree to keep the standard lockfiles in your branch unchanged.

The patch pins AccessKit 0.25.1 and accesskit_macos 0.27.1. Each native content
view receives an adapter before first responder assignment. Platform callbacks
only queue actions; the UI thread validates window, owner generation, live
control, and supported action before using the ordinary widget action path.
Adapters retire before the native view, and native notification batches are
raised after releasing bridge borrows. Trees published at the end of a draw and
committed keyboard-focus changes reach AppKit even when the window is resting.
Native activation does not queue a competing scroll; explicit native focus
reveals its target. The patch also exposes dropdown popup rows, keys TextInput's
retained layout on typography (enlarging text keeps text, selection, history
and scroll), and adds `View::get_scroll_pos` for the actual scrollbar offset.

## Build

```sh
cargo build --release --features owner-accessibility --config .run/owner-daemon/native-accessibility.cargo.toml
```

## Isolated tests

```sh
cargo test --manifest-path crates/owner-dashboard/Cargo.toml --features native-accessibility,fixtures --config .run/owner-daemon/native-accessibility.cargo.toml --lib
CARGO_TARGET_DIR=.run/owner-daemon/targets/accessibility-embedded-release cargo build --release --example miniapp_dock --features owner-accessibility,onboarding-test-support --config .run/owner-daemon/native-accessibility.cargo.toml
DOCK=.run/owner-daemon/targets/accessibility-embedded-release/release/examples/miniapp_dock
python3 tools/owner-dashboard-ax.py --binary "$DOCK" --output .run/owner-daemon/native-ax
python3 tools/owner-dashboard-host-scale.py --binary "$DOCK" --output .run/owner-daemon/ux/host-scale
python3 tools/owner-dashboard-matrix.py --binary "$DOCK" --width 420 --scale 2 --output .run/owner-daemon/ux/matrix-420-200
python3 tools/owner-dashboard-pane.py --binary "$DOCK" --width 768 --height 720 --output .run/owner-daemon/ux/pane-768x720
```

The `miniapp_dock` example embeds the dashboard in Desktop's dock. Each runner
launches and closes its own hidden native window with an isolated offline
profile and drives the dashboard fixture through the example's `dock:` remote
events. The runners do not inspect another app, start hosting, connect to real
accounts, or enable VoiceOver. Retained JSON reports record the exact executable
and its SHA-256; `host-scale` needs Pillow.

- `owner-dashboard-ax.py` queries the fixture's own AppKit NSAccessibility
  objects and acts through native press and focus APIs: Agent navigation,
  selected tabs, Retina bounds, stable refresh identity, command and
  confirmation paths with focus restoration, unknown metrics, offscreen focus
  reveal, and owner switching. It separates semantic-tree from native evidence.
- `owner-dashboard-host-scale.py` holds dashboard scale at 1 and compares
  rendered glyphs at host typography 1 and 1.5, so an injected host scale must
  change actual text rather than only an environment variable.
- `owner-dashboard-matrix.py` captures English/Chinese and light/dark pages,
  fixture states, ten-refresh draft/focus/geometry retention, filter input
  acknowledgment latency and the paged resource picker. `--scale` sets dashboard
  text scale, `--host-scale` the host typography and `--zoom` the window DPI
  override (`dock:owner-zoom:F`); no system display setting changes.
- `owner-dashboard-pane.py` resizes the real native window until the dock pane
  has the requested size and checks the measured dashboard bounds. The
  production dock minimum is 420 logical pixels; `--width 360 --height 640
  --scale 2 --component` constrains the dashboard instance through Makepad's
  tweaker, and the report distinguishes that component test from production
  dock sizing.

Native API tests are not a manual VoiceOver acceptance journey. Full VoiceOver
reading/navigation, spoken announcements and magnifier behavior still require a
separate user-operated assistive-technology review. No UX score follows from
passing these tests alone.
