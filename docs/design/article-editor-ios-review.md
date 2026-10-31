# Article editor: iOS style and native instrumentation

Scope: the production article mini app, including its library, Markdown editor,
preview, formatting controls and style selection. Preserve draft storage,
consent, publication confirmation and dock/undock behavior.

Acceptance target: **at least 95/100**, assessed after inspecting actual native
screenshots. The instrument reports assertions, not a visual score. Every failed
interaction or clipped primary action blocks acceptance regardless of the score.

| Area | Weight | Evidence required |
| --- | ---: | --- |
| Hierarchy and iOS appearance | 25 | Clear navigation, labeled segments, explicit primary action, restrained surfaces and rounded groups |
| Responsive layout | 20 | 390, 760 and 1120+ point content widths; no clipped controls or preview spilling out of its pane |
| Theme and legibility | 20 | Light/dark screenshots, semantic chrome colors, readable selected and inactive controls |
| Editing and state continuity | 25 | Real native typing, draft save, mode/style changes, keyboard editing, no autosave layout movement |
| Reachability | 10 | At least 44-point primary targets, visible labels, 130% text and retained keyboard focus |

Use `examples/article_editor_review.rs` with `onboarding-test-support` and
`tools/article-editor/native_ios_review.py`. The process uses the real ArticlePanel
and consent/storage handlers with a synthetic offline Matrix identity. Its explicit
profile marker prevents accidental use of an existing login. No sync or publishing
is started. Capture uses hidden native Makepad/Metal windows, not mock HTML or a
GPU-free renderer. iOS appearance is reviewed on macOS; actual iPhone keyboard and
safe-area behavior require device testing and are not claimed here.

Baseline: icon-only mode/publication controls, hard-coded white/green segment
colors, 36-point format targets, and a fixed 420-point preview sheet that can
overflow when the inspector is open. Baseline capture is retained in
`.run/article-ios`.

Final review: **96/100**, implementer self-assessment against the rubric above.
This is a visual and interaction review of the tested native views, not an
independent rating or certification of the whole application.

| Area | Result | Review and remaining deduction |
| --- | ---: | --- |
| Hierarchy and iOS appearance | 24/25 | Labeled segmented navigation, clear publication action, quiet surfaces, bottom style sheet. The style action still uses a palette symbol rather than a permanent text label. |
| Responsive layout | 19/20 | Four widths pass. Preview fits beside the inspector; navigation expands with font scale; long library titles retain their status. Wide source-only mode still uses the full editor width, giving long prose lines. |
| Theme and legibility | 19/20 | Light/dark chrome and article quotes remain readable; disabled publication is visibly gray. Preset captions are smaller than editor text. |
| Editing and state continuity | 25/25 | Native input, save, formatting selection, undo/redo, style cancellation, mode/theme changes and reopening a saved draft pass. Autosave does not move the source or navigation. |
| Reachability | 9/10 | Primary targets are at least 44 points; English controls remain complete at 130% font scale. Some secondary formatting actions require horizontal scrolling in compact layouts. |

The final instrument ran five cases: Chinese at 390 and 1120 points, English at
390, 760 and 1440 points, each 820 points high. All **65 assertions passed**, with
no detected runtime panic, script error or shader compilation failure. Publication
tests stop at the cover/destination or review page; no article is sent. The empty
destination state explains the missing chat and disables publication.

These are copies of real native captures, inspected during review:

- [Light editor](article-editor-ios/editor-light.png) and [dark editor](article-editor-ios/editor-dark.png).
- [130% English navigation](article-editor-ios/editor-large-text.png) and [wrapped library title](article-editor-ios/library-large-text.png).
- [Dark style sheet](article-editor-ios/style-sheet-dark.png) and [desktop split with inspector](article-editor-ios/desktop-split.png).
- [Publication review and empty destination](article-editor-ios/publish-review.png).
- [Native assertion results and capture hashes](article-editor-ios/native-review.json).

The instrument exposed and verified fixes for 43-point inherited button margins,
lost editor focus after formatting (breaking keyboard undo), dark quote fills on
light article paper, clipped English actions at large font scale, and library
status lines clipped beneath wrapped titles. Screenshot inspection remains
necessary: widget bounds alone did not detect truncated text.

Reproduce with a fresh output directory; the fixture must not reuse a real login:

```sh
cargo build --locked --features onboarding-test-support --example article_editor_review
python3 tools/article-editor/native_ios_review.py \
  --binary target/debug/examples/article_editor_review \
  --output .run/article-ios/new-review --width 390 --language en
```

Full traces, all screenshots, failed diagnostic runs and build logs remain in
`.run/article-ios` in the build worktree. The shared release gate also includes
the article editor's existing model, storage and interaction unit tests.

Final release gate: **58 checks and 317 Rust tests passed**. Release build completed
with `--locked --features onboarding-test-support --bin rinx`.
