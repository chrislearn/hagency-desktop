This is a release candidate for Hagency Desktop and macOS resource hosting.

- Windows: update Makepad to `4ec079b02bca9747acde3216cfc370cd558e40f4`, containing both fixes in [Makepad #132](https://github.com/OctoSense-org/makepad/pull/132): mouse activation restores the correct window focus, and secondary windows repaint while the main window keeps refreshing. The dependency is pinned to the PR author's published repository while the PR awaits merge.
- Hosting: the separate **Hagency Hosting** app includes the resource daemon. Enable hosting after signing in as the resource owner; the daemon continues after closing Desktop or the Hosting window.
- Hosting language: use the Language selector at the top of the window to choose **System default**, **English**, or **简体中文**. The selection applies immediately to the dashboard and setup and survives restarting the app. The embedded dashboard follows Desktop's language setting.
- Desktop: clearer Agent permission choices, a single message-response settings entry, smaller navigation arrows, compact token summaries, and replies that use the available chat width.

Install **Hagency Desktop** for chat and Agent management. Resource owners using macOS can additionally install **Hagency Hosting** to keep execution available independently of the chat app. Provider accounts and coding tools still need to be configured on the execution machine.

Windows native resource hosting (including the background daemon), Windows ARM64, automatic updates, and mobile store packages are not included. Windows web links use the external-browser option; native article tabs and the editor remain available. This RC does not claim a new manual Windows 11 test of the complete installer; the upstream PR's Windows 11 verification and the build/test results are distinct evidence.

The version series is intentionally reset from inherited Rinx 1.1.x releases to Hagency 0.5.0. Install this RC manually. The earlier [dependency audit follow-up](https://github.com/hagency-org/hagency-desktop/blob/work/hagency-desktop-local-20261028/dev-vibe/release-0.5.0/assessment.zh.md) remains open; this RC does not claim that all upstream dependency advisories are resolved.
