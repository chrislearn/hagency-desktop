# Native macOS accessibility build and verification

The shared `owner_dashboard::accessibility` module composes dashboard and host
controls into one tree per native window. Each host calls `after_event` once,
after the complete UI handles the event. Custom host controls register a
`ControlAdapter`; Desktop registers its navigation rail, including Agent.

## Reproducible source preparation

```sh
python3 tools/prepare-owner-accessibility.py --output-config .run/owner-daemon/native-accessibility.cargo.toml
```

This creates an isolated Makepad checkout at
`ec0283bfd78cb1634788e86a6b26e708980499dd`, applies the tracked
`makepad-accessibility.patch`, and writes a Cargo configuration with absolute
paths. The directory name includes the patch SHA-256. Reuse verifies HEAD and
all changed file hashes and rejects unexpected edits. User Makepad checkouts and
Cargo git caches are read only. Historical Desktop Makepad source aliases are
patched directly because Cargo patches are not transitive.

The patch pins AccessKit 0.25.1 and accesskit_macos 0.27.1. Each native content
view receives an adapter before first responder assignment. Platform callbacks
only queue actions; the UI thread validates window, owner generation, live
control, and supported action before using the ordinary widget action path.
Adapters retire before the native view. Returned native notification batches
are raised after releasing bridge borrows. A native wake commits trees published
at the end of a draw even when the window is otherwise resting.
Committed keyboard-focus events also publish an update, since focus requested
during layout becomes current only after the draw handler returns.
Native activation keeps the normal control action without queuing a competing
scroll to the activated button. Explicit native focus reveals its target; a
subsequent validation or confirmation can reveal the new focus instead.

The patch also exposes actual dropdown popup rows for accessibility and fixes
TextInput's retained layout key to include typography. The latter preserves text,
selection, history and scroll when users enlarge text or change font styling.
`View::get_scroll_pos` exposes the actual scrollbar offset so host handover does
not depend on a page's position below transient notices or inventory banners.

## Build

Native accessibility is an explicit patched-framework build, not part of the
ordinary fixed-Git Makepad build. `packaging/owner-controller/build.py` prepares
an isolated `native-source` copy for its accessibility lockfiles, verifies that
the helper checkout is clean and matches Desktop's published Client pin, and
leaves all three standard workspace lockfiles unchanged. For manual commands
below, use a disposable checkout/worktree so path-patch resolution does not
replace the standard lockfiles in your working branch.

Standalone production:

```sh
cargo build --release --manifest-path apps/owner-controller/Cargo.toml --features native-accessibility --config .run/owner-daemon/native-accessibility.cargo.toml
```

Desktop production adds `--features owner-accessibility` and the same configuration.
A build without the feature has an explicit no-op semantic publisher and must not
be presented as native accessibility verified.

## Isolated tests

```sh
cargo test --manifest-path crates/owner-dashboard/Cargo.toml --features native-accessibility,fixtures --config .run/owner-daemon/native-accessibility.cargo.toml --lib
CARGO_TARGET_DIR=.run/owner-daemon/targets/accessibility-release cargo build --release --manifest-path apps/owner-controller/Cargo.toml --features fixtures,native-accessibility --config .run/owner-daemon/native-accessibility.cargo.toml
python3 tools/owner-dashboard-ax.py --binary .run/owner-daemon/targets/accessibility-release/release/hagency-owner-controller --output .run/owner-daemon/native-ax-standalone
CARGO_TARGET_DIR=.run/owner-daemon/targets/accessibility-embedded-release cargo build --release --example miniapp_dock --features owner-accessibility,onboarding-test-support --config .run/owner-daemon/native-accessibility.cargo.toml
python3 tools/owner-dashboard-ax.py --binary .run/owner-daemon/targets/accessibility-embedded-release/release/examples/miniapp_dock --embedded --output .run/owner-daemon/native-ax-embedded
```

The runner launches and closes its own hidden native Metal fixture. Embedded
runs use an isolated offline profile. Fixture-only probes query AppKit
NSAccessibility objects belonging to that process; actions use native press,
value, focus and selection APIs. They do not inspect another app, start hosting,
connect to real accounts, or enable VoiceOver. The runner requires Pillow for
RGB comparisons of the native reduced-motion frames. Retained JSON artifacts distinguish
the semantic tree from native AppKit evidence and include the exact executable.

The script exercises selected tabs, bounds at Retina scale, normal command and
confirmation paths, unknown metrics, stable refresh identity, owner clearing,
and setup text editing. It also invokes the fixture NSView's real
NSTextInputClient marked-text and commit APIs, verifies a non-Latin commit keeps
the existing text, compares three reduced-motion native frames, and measures
populated-field glyph growth when text is enlarged to 200%. Text coordinates use grapheme clusters internally and
native UTF-16 offsets at the AppKit boundary. Password input values are masked
and have no plaintext text-run children.

Native API tests are not a manual VoiceOver acceptance journey. Full VoiceOver
reading/navigation, spoken announcements and magnifier behavior still require a
separate user-operated assistive-technology review. No UX score follows from
passing these tests alone.

## Compact layout and scroll handover

```sh
python3 tools/owner-dashboard-compact.py --binary .run/owner-daemon/targets/accessibility-release/release/hagency-owner-controller --output .run/owner-daemon/ux/compact-en-200
python3 tools/owner-dashboard-compact.py --binary .run/owner-daemon/targets/accessibility-release/release/hagency-owner-controller --locale zh --output .run/owner-daemon/ux/compact-zh-200
python3 tools/owner-dashboard-compact.py --binary .run/owner-daemon/targets/accessibility-release/release/hagency-owner-controller --scale 1 --host-scale 1.5 --output .run/owner-daemon/ux/compact-host-150
```

This isolated journey checks the initial total (including an explicitly unknown
total when a provider omits a token category), owner disclosure identity,
readable resource rows, both confirmation actions and context, uncertain-command
recovery, and scroll restoration while notice and inventory details occupy the
footer and the persistent status strip remains visible. It also checks real
ArrowDown focus, viewport reveal, and Return selection for a long account name.
Names taller than the viewport are revealed from their beginning and remain
scrollable. It retains rendered PNGs, control bounds and the exact binary hash. The host-scale argument sets
the fixture stylesheet's typography multiplier independently of dashboard text
scaling. `zoom:F` (standalone) and `dock:owner-zoom:F` (embedded fixture) separately
exercise the window DPI override; they do not change any system display setting.

To verify that an injected host scale actually changes rendered text, rather
than only recording an environment variable, compare populated glyphs from the
same executable with dashboard scale held at 1:

```sh
python3 tools/owner-dashboard-host-scale.py --binary .run/owner-daemon/targets/accessibility-release/release/hagency-owner-controller --output .run/owner-daemon/ux/host-scale-proof
python3 tools/owner-dashboard-host-scale.py --binary .run/owner-daemon/targets/accessibility-embedded-release/release/examples/miniapp_dock --embedded --output .run/owner-daemon/ux/embedded-host-scale-proof
```

`tools/owner-dashboard-pane.py` uses the embedded fixture's real native window
resize hook and checks the measured dashboard bounds. For example:

```sh
python3 tools/owner-dashboard-pane.py --binary .run/owner-daemon/targets/accessibility-embedded-release/release/examples/miniapp_dock --width 768 --height 720 --output .run/owner-daemon/ux/embedded-pane-768x720
```

The production dock minimum is 420 logical pixels. `--width 360 --height 640
--scale 2 --component` explicitly constrains the existing dashboard instance
through the fixture's Makepad tweaker. The report distinguishes that component
test from production dock sizing and records both parent and dashboard bounds.
