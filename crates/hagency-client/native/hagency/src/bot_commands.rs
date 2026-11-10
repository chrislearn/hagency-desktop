//! Bot command layer, ported 1:1 from the retained bridge.
//!
//! Source of truth: `lib/bot-commands.js` — the tier table and `classifyCommand`
//! (:30-59), the ACL `authorizeCommand` (:84-101), `parse` (:306-310), the
//! dispatch switch and its unknown-command reply (:364-389), the no-parse reply
//! (:324-326), `!offer` (:469-524), `!help` (:630-693) and the tier-1 reads
//! `!status`/`!agents`/`!sessions` (:694-836) — plus the command branch of
//! `bridge-matrix.js:7111-7125` that keeps a `!` line out of agent input.
//!
//! This module is PURE. It parses, authorizes and renders; it performs no IO,
//! holds no store, opens no transport. That is what lets every user-visible
//! string the retained product printed be asserted without a homeserver: the
//! caller supplies the observations TS fetched over HTTP (`/api/agents`,
//! `/api/groups`, `/api/offer-book`) and from tmux. Where native has no such
//! source at all — native has no tmux model — the retained product's own
//! "no source" branch is the faithful answer, and that branch is named at each
//! site below rather than invented.

use serde_json::{Value, json};
use std::collections::BTreeSet;

/// One command reply, in the retained product's own shape: a plain body plus the
/// optional `org.matrix.custom.html` rendering (`reply`, :397-404). The refusals
/// TS sent without html stay `html: None`, which is exactly what makes them
/// single-line in a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub plain: String,
    pub html: Option<String>,
}

impl Reply {
    fn text(plain: impl Into<String>) -> Self {
        Self {
            plain: plain.into(),
            html: None,
        }
    }
    fn rich(plain: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            plain: plain.into(),
            html: Some(html.into()),
        }
    }
    /// The exact Matrix content `reply` builds (:397-404): always `m.text`, and
    /// `format` + `formatted_body` only when the handler rendered html.
    pub fn content(&self) -> Value {
        match &self.html {
            None => json!({"msgtype": "m.text", "body": self.plain}),
            Some(html) => json!({
                "msgtype": "m.text",
                "body": self.plain,
                "format": "org.matrix.custom.html",
                "formatted_body": html,
            }),
        }
    }
}

/// `escHtml` (:129-131).
fn esc_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── Command ACL ─────────────────────────────────────────────────────
//
// The table is `COMMAND_TIERS` (:30-59) verbatim, including the two tier-0
// comments' conclusion: `!help`, `!request` and `!offer` are public by design.
// `!request` is dispatched elsewhere (it is a project-side act carried as an
// event, ADR-095), but its tier stays here so the table is the table.
pub const COMMAND_TIERS: &[(&str, u8)] = &[
    ("!help", 0),
    ("!request", 0),
    ("!offer", 0),
    ("!status", 1),
    ("!agents", 1),
    ("!groups", 1),
    ("!group", 1),
    ("!agent", 1),
    ("!sessions", 1),
    ("!mcp", 1),
    ("!bridge", 1),
    ("!mkgroup", 2),
    ("!bindroom", 2),
    ("!addmember", 2),
    ("!rmember", 2),
    ("!joingroup", 2),
    ("!dm", 2),
    ("!identity", 2),
    ("!rmgroup", 2),
    ("!spy", 3),
    ("!agentctl", 3),
    ("!ctl", 3),
];

/// `classifyCommand` (:61-63): an unknown command defaults to OPERATOR tier.
pub fn classify(command: &str) -> u8 {
    COMMAND_TIERS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, tier)| *tier)
        .unwrap_or(1)
}

/// The ACL lists and the unconfigured-override, read exactly as :20-27 and
/// :84-101 do.
#[derive(Debug, Clone, Default)]
pub struct Acl {
    operator: BTreeSet<String>,
    admin: BTreeSet<String>,
    /// `MATRIX_ALLOW_UNCONFIGURED_ACL`, matched case-insensitively against the
    /// literal `true` (:83-84).
    allow_unconfigured: bool,
}

impl Acl {
    pub fn new(
        operator: impl IntoIterator<Item = String>,
        admin: impl IntoIterator<Item = String>,
        allow_unconfigured: bool,
    ) -> Self {
        Self {
            operator: operator.into_iter().collect(),
            admin: admin.into_iter().collect(),
            allow_unconfigured,
        }
    }

    /// The deployment's lists from the environment, split on `,`, trimmed, empty
    /// entries dropped (:20-27).
    pub fn from_env() -> Self {
        let list = |name: &str| {
            std::env::var(name)
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let allow = std::env::var("MATRIX_ALLOW_UNCONFIGURED_ACL")
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            == "true";
        Self::new(
            list("MATRIX_OPERATOR_MXIDS"),
            list("MATRIX_ADMIN_MXIDS"),
            allow,
        )
    }

    /// Whether `sender` is an operator, for the `/thread` directive gate
    /// (TS `msg.trustLevel === 'operator'`, backend-v2.js:2265). Admin is not
    /// operator: the directive answers only to the operator list, exactly as TS
    /// derives `trustLevel` from `MATRIX_OPERATOR_MXIDS` alone.
    pub fn is_operator(&self, sender: &str) -> bool {
        self.operator.contains(sender)
    }

    /// `authorizeCommand` (:84-101). `Ok(reason)` carries the reason TS logged;
    /// `Err(reason)` is the refusal key the caller turns into words. Tier 0 is
    /// decided first and is never refused.
    pub fn authorize(&self, sender: &str, tier: u8) -> Result<&'static str, &'static str> {
        if tier == 0 {
            return Ok("public");
        }
        if self.admin.contains(sender) {
            return Ok("admin");
        }
        if tier <= 2 && self.operator.contains(sender) {
            return Ok("operator");
        }
        if self.operator.is_empty() && self.admin.is_empty() {
            // AN UNCONFIGURED ACL FAILS CLOSED (:64-83). The permissive branch
            // exists only for the explicit `MATRIX_ALLOW_UNCONFIGURED_ACL=true`
            // opt-in, and it is reached only when nothing is configured — the
            // case nobody tests.
            return if self.allow_unconfigured {
                Ok("no_acl")
            } else {
                Err("acl_unconfigured")
            };
        }
        Err(if tier >= 3 {
            "admin_required"
        } else {
            "operator_required"
        })
    }
}

/// `parse` (:306-310): a leading `!` after trimming, first token lowercased, the
/// rest are arguments. Anything not starting with `!` is not a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub command: String,
    pub args: Vec<String>,
}

pub fn parse(text: &str) -> Option<Parsed> {
    let trimmed = text.trim();
    if !trimmed.starts_with('!') {
        return None;
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    Some(Parsed {
        command: parts[0].to_lowercase(),
        args: parts[1..].iter().map(|arg| (*arg).to_owned()).collect(),
    })
}

// ── Texts ───────────────────────────────────────────────────────────
/// `Send !help for available commands.` — the one-liner the dispatch sends when
/// the text is not a command at all (:324-326).
pub fn no_command_reply() -> Reply {
    Reply::text("Send !help for available commands.")
}

/// The dispatch `default:` arm (:388).
pub fn unknown_command(command: &str) -> Reply {
    Reply::text(format!(
        "Unknown command: {command}\nSend !help for available commands."
    ))
}

/// The `tier0Only` refusal (:338-345): a bridge running with no bot cannot
/// half-succeed a privileged command, so it refuses before any mutation and
/// names the fix.
pub fn tier0_only_refusal(command: &str) -> Reply {
    Reply::text(format!(
        "{command} is unavailable: this Hagency is running without a Matrix bot, so only \
         public commands (!help, !offer, !request) work here. Configure \
         MATRIX_BOT_PASSWORD to enable the rest."
    ))
}

/// The ACL refusal (:352-361). The unconfigured case gets its own message:
/// "requires operator privileges" is unactionable when the list is empty and
/// nobody can ever satisfy it.
pub fn acl_refusal(command: &str, reason: &str) -> Reply {
    if reason == "acl_unconfigured" {
        Reply::text(format!(
            "Access denied: {command} is privileged and no operator ACL is configured on this \
             Hagency. Set MATRIX_OPERATOR_MXIDS (or MATRIX_ADMIN_MXIDS) to the humans who may \
             run it."
        ))
    } else {
        Reply::text(format!(
            "Access denied: {command} requires {} privileges.",
            if reason == "admin_required" {
                "admin"
            } else {
                "operator"
            }
        ))
    }
}

/// `!help` (:630-693), plain body and html, captured from the source verbatim.
pub const HELP_PLAIN: &str = "=== Agent Bridge Bot Commands ===\n\nAsking for capacity (any member of a project room):\n  !offer                              — what is on offer, and what would serve it\n  !request <role> <tokens> [per-day]  — ask this contributor for an agent\n                                        e.g. !request architect 400000 20000\n\nSystem:\n  !status          — System overview\n  !agents          — List online agents (!agents all for full list)\n  !groups          — List all groups\n  !sessions        — Tmux sessions + current process\n  !mcp             — MCP status per session\n  !bridge          — Bridge internal state\n  !ctl ...         — Agent control in agent DM (status/send/key)\n\nDetail:\n  !agent [name]    — Agent details (auto in DM)\n  !group [name]    — Group details (auto in group)\n  !agentctl <agent> status [N]|send <text>|key <K> — Control agent pane\n\nManagement:\n  !mkgroup <name> <m1> <m2> ...  — Create group\n  !bindroom <group>              — Bind THIS room to an existing group\n  !addmember [group] <name>      — Add member to group\n  !rmember [group] <name>        — Remove member from group\n  !rmgroup [group]               — Delete group + Matrix room\n  !joingroup [group]             — Join a group yourself\n  !dm <agent>                    — Create DM room with agent\n  !identity [agent] <text>       — Set agent identity (auto in DM)\n  !spy <agent1> <agent2>         — Join an agent DM room to watch";
pub const HELP_HTML: &str = "<h3>Agent Bridge Bot Commands</h3><b>System:</b><br><code>!status</code> — System overview<br><code>!agents</code> — List online agents (<code>!agents all</code> for full list)<br><code>!groups</code> — List all groups<br><code>!sessions</code> — Tmux sessions + current process<br><code>!mcp</code> — MCP status per session<br><code>!bridge</code> — Bridge internal state<br><code>!ctl ...</code> — Agent control in agent DM (status/send/key)<br><br><b>Detail:</b><br><code>!agent [name]</code> — Agent details (auto in DM)<br><code>!group [name]</code> — Group details (auto in group)<br><code>!agentctl &lt;agent&gt; status [N]|send &lt;text&gt;|key &lt;K&gt;</code> — Control agent pane<br><br><b>Management:</b><br><code>!mkgroup &lt;name&gt; &lt;m1&gt; &lt;m2&gt; ...</code> — Create group<br><code>!bindroom &lt;group&gt;</code> — Bind THIS room to an existing group<br><code>!addmember [group] &lt;name&gt;</code> — Add member to group<br><code>!rmember [group] &lt;name&gt;</code> — Remove member from group<br><code>!rmgroup [group]</code> — Delete group + Matrix room<br><code>!joingroup [group]</code> — Join a group yourself<br><code>!dm &lt;agent&gt;</code> — Create DM room with agent<br><code>!identity [agent] &lt;text&gt;</code> — Set agent identity (auto in DM)<br><code>!spy &lt;agent1&gt; &lt;agent2&gt;</code> — Join an agent DM room to watch<br>";

pub fn help() -> Reply {
    Reply::rich(HELP_PLAIN, HELP_HTML)
}

// ── Tier-1 reads ────────────────────────────────────────────────────

/// The observations `!status` reads: `/api/agents` and `/api/groups` sizes, and
/// tmux (:694-732). `sessions: None` is TS's `sessionCount === null`, which is
/// what `hasTmuxBinary()` false or a dead tmux server produced.
#[derive(Debug, Clone, Default)]
pub struct StatusObservation {
    pub agents: usize,
    pub groups: usize,
    pub sessions: Option<usize>,
    pub tmux_note: Option<String>,
}

pub fn status(observed: &StatusObservation) -> Reply {
    let session_count = match observed.sessions {
        Some(count) => count.to_string(),
        None => "unavailable".to_owned(),
    };
    let plain = format!(
        "=== System Status ===\nAgents: {}\nGroups: {}\nTmux sessions: {}\nBridge: running",
        observed.agents, observed.groups, session_count
    );
    let plain = match &observed.tmux_note {
        Some(note) => format!("{plain}\nTmux note: {note}"),
        None => plain,
    };
    let html = format!(
        "<b>System Status</b><br>Agents: <b>{}</b><br>Groups: <b>{}</b><br>Tmux sessions: <b>{}</b><br>Bridge: <b>running</b>",
        observed.agents, observed.groups, session_count
    );
    let html = match &observed.tmux_note {
        Some(note) => format!("{html}<br>Tmux note: <i>{}</i>", esc_html(note)),
        None => html,
    };
    Reply::rich(plain, html)
}

/// One `/api/agents` row as `!agents` reads it: a name and an optional identity
/// (:733-801).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentItem {
    pub name: String,
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentsObservation {
    pub agents: Vec<AgentItem>,
    /// The tmux session names, when tmux answered. `None` is "unknown", which is
    /// NOT the same as "offline": it renders `?` and keeps every agent listed.
    pub sessions: Option<BTreeSet<String>>,
    pub tmux_note: Option<String>,
}

pub fn agents(observed: &AgentsObservation, show_all: bool) -> Reply {
    if observed.agents.is_empty() {
        return Reply::text("No known agents yet.");
    }
    let mut lines = vec![if show_all {
        "=== All Agents ===".to_owned()
    } else {
        "=== Online Agents ===".to_owned()
    }];
    let mut html_lines = vec![if show_all {
        "<b>All Agents</b><br><br>".to_owned()
    } else {
        "<b>Online Agents</b><br><br>".to_owned()
    }];
    let mut filtered = 0usize;
    for agent in &observed.agents {
        let alive = observed
            .sessions
            .as_ref()
            .map(|sessions| sessions.contains(&agent.name));
        if !show_all && alive == Some(false) {
            filtered += 1;
            continue;
        }
        let (icon, colour) = match alive {
            None => ("?", "#ffd43b"),
            Some(true) => ("●", "#69db7c"),
            Some(false) => ("○", "#888"),
        };
        let id_text = agent
            .identity
            .as_deref()
            .map(|identity| format!(" — {identity}"))
            .unwrap_or_default();
        lines.push(format!("{icon} {}{id_text}", agent.name));
        let id_html = agent
            .identity
            .as_deref()
            .map(|identity| format!(" — <i>{}</i>", esc_html(identity)))
            .unwrap_or_default();
        html_lines.push(format!(
            "<span style=\"color:{colour}\">{icon}</span> <b>{}</b>{id_html}<br>",
            esc_html(&agent.name)
        ));
    }
    if let Some(note) = &observed.tmux_note {
        lines.push(String::new());
        lines.push(format!("Tmux note: {note}"));
        html_lines.push(format!("<br>Tmux note: <i>{}</i>", esc_html(note)));
    }
    if !show_all && filtered > 0 {
        lines.push(String::new());
        lines.push("Use !agents all to see all agents including offline.".to_owned());
        html_lines.push(
            "<br><i>Use <code>!agents all</code> to see all agents including offline.</i>"
                .to_owned(),
        );
    }
    Reply::rich(lines.join("\n"), html_lines.join(""))
}

/// One tmux session as `!sessions` reads it (:802-836). Native has no tmux
/// model at all, so in production the caller reports the retained product's own
/// no-binary branch (`tmux is not installed on bridge host.`) rather than
/// inventing sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionItem {
    pub name: String,
    pub process: String,
}

/// `!sessions` when tmux cannot be reached. `installed` distinguishes TS's two
/// different first answers (:803-812); both are single-line and html-less.
pub fn sessions_unavailable(installed: bool) -> Reply {
    if installed {
        Reply::text("No tmux sessions found (tmux server not running).")
    } else {
        Reply::text("tmux is not installed on bridge host.")
    }
}

pub fn sessions(observed: &[SessionItem]) -> Reply {
    if observed.is_empty() {
        return Reply::text("No tmux sessions found.");
    }
    let mut lines = vec!["=== Tmux Sessions ===".to_owned()];
    let mut html_lines = vec!["<b>Tmux Sessions</b><br><br>".to_owned()];
    for session in observed {
        lines.push(format!("  {}: {}", session.name, session.process));
        html_lines.push(format!(
            "<code>{}</code>: <b>{}</b><br>",
            esc_html(&session.name),
            esc_html(&session.process)
        ));
    }
    lines.push(format!("\nTotal: {}", observed.len()));
    html_lines.push(format!("<br>Total: <b>{}</b>", observed.len()));
    Reply::rich(lines.join("\n"), html_lines.join(""))
}

// ── !offer (tier 0) ─────────────────────────────────────────────────

/// The `GET /api/offer-book` projection `!offer` reads (:469-524). `whitelisted`
/// is tri-state on purpose: `None` means no room was identified, which is not
/// the same as "not trusted", so it is not reported as a refusal.
#[derive(Debug, Clone, Default)]
pub struct OfferBook {
    /// When `Some`, the read itself failed and `!offer` says so instead of
    /// pretending nothing is published.
    pub error: Option<String>,
    pub roles: Vec<OfferRole>,
    pub whitelisted: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct OfferRole {
    pub role: String,
    pub serving: Option<OfferServing>,
    pub budget_cap_per_engagement: Option<u64>,
    pub rate_cap: Option<String>,
    pub count: Option<u64>,
    pub running_now: Option<u64>,
}

/// The transparency ruling, in the place a borrower reads it: framework, model,
/// level, tier.
#[derive(Debug, Clone, Default)]
pub struct OfferServing {
    pub framework: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub tier: Option<String>,
    pub provisioning_required: bool,
}

/// `!request` (board #79; TS `lib/bot-commands.js:525-629` `cmdRequest`).
/// What the host learned from submitting the request into the engagement
/// intake — the fields the TS reply text renders. `project` is the room NAME
/// (the label), never the id; the id never changes identity or authority.
#[derive(Debug, Clone, Default)]
pub struct RequestOutcome {
    /// The backend's refusal (`result.error`), rendered as `Request refused: …`.
    pub error: Option<String>,
    /// The engagement the intake returned. `None` when the submit produced no
    /// engagement at all (`Request failed: no engagement returned.`).
    pub engagement: Option<RequestEngagement>,
    /// The serving configuration for the transparency line; `None` when the
    /// agent record is gone (degrades to the agent alone, never fabricated).
    pub serving: Option<OfferServing>,
    /// `binding.bound === false`: the attach did not happen. The project is
    /// told THAT, never the provider's remedy (`HAGENCY_*` stays private).
    pub attach_failed: bool,
}

#[derive(Debug, Clone)]
pub struct RequestEngagement {
    pub role: String,
    pub requested_tokens: u64,
    pub allocated_tokens: u64,
    pub agent: String,
    pub auto_joined: bool,
    /// The TS `route` word deciding the pending reply's WHY.
    pub route: Option<String>,
}

/// `cmdRequest` (:525-629), the reply half. The argument validation is
/// synchronous exactly as TS: usage and malformed-token refusals never reach
/// the backend.
pub fn request_reply(args: &[String], outcome: Option<&RequestOutcome>) -> Reply {
    let usage = "Usage: !request <role> <tokens> [tokens-per-day]\n\
                 Example: !request architect 400000 20000";
    let Some(_role) = args.first().filter(|role| !role.is_empty()) else {
        return Reply::text(usage);
    };
    let Some(tokens_raw) = args.get(1) else {
        return Reply::text(usage);
    };
    // `Number(String(tokensRaw).replace(/[_,]/g, ''))` (:533-536).
    let cleaned: String = tokens_raw
        .chars()
        .filter(|c| *c != '_' && *c != ',')
        .collect();
    let parsed = cleaned.parse::<u64>();
    let Ok(requested_tokens) = parsed else {
        return Reply::text(format!("Not a token amount: {tokens_raw}"));
    };
    if requested_tokens == 0 {
        return Reply::text(format!("Not a token amount: {tokens_raw}"));
    }
    let Some(outcome) = outcome else {
        // The caller did not submit (or has no intake): TS never rendered
        // anything without calling the backend, so this is the refusal shape.
        return Reply::text("Request failed: no engagement returned.");
    };
    if let Some(error) = &outcome.error {
        return Reply::text(format!("Request refused: {error}"));
    }
    let Some(e) = &outcome.engagement else {
        return Reply::text("Request failed: no engagement returned.");
    };
    if e.auto_joined {
        // The transparency ruling (:596-612): the agent and its configuration
        // are disclosed to the borrower; the provider's deployment is not.
        let config = outcome.serving.as_ref().and_then(|s| {
            let model = s.model.as_deref()?;
            let mut text = format!("{} · {}", s.framework.as_deref().unwrap_or("?"), model);
            if let Some(reasoning) = &s.reasoning {
                text.push_str(&format!(" ({reasoning})"));
            }
            if let Some(tier) = &s.tier {
                text.push_str(&format!(" · {tier}"));
            }
            Some(text)
        });
        let mut plain = format!(
            "Joined automatically as {} — {} tokens, served by {}",
            e.role, e.allocated_tokens, e.agent
        );
        plain.push_str(&match config {
            Some(config) => format!(" running {config}."),
            None => ".".to_owned(),
        });
        if outcome.attach_failed {
            plain.push_str(
                "\nNote: the agent could not be attached yet — the contributor has been notified.",
            );
        }
        return Reply::text(plain);
    }
    // WHY it is waiting, in the project's own terms (:619-628).
    let why = match e.route.as_deref() {
        Some("notWhitelisted") => "this room is not on the contributor's whitelist",
        Some("overOffer") => "the amount is above what they have published",
        Some("overCeiling") => "the amount is above what the serving agent has left",
        _ => "it needs a decision",
    };
    Reply::text(format!(
        "Requested {} for {} tokens — awaiting a decision, because {}.",
        e.role, e.requested_tokens, why
    ))
}

pub fn offer(book: &OfferBook) -> Reply {
    if let Some(error) = &book.error {
        return Reply::text(format!("Cannot read the offer book: {error}"));
    }
    if book.roles.is_empty() {
        // Distinguished from an error on purpose: "nothing published" is a real
        // and common state, and reporting it as a failure would send a project
        // to ask why the bot is broken.
        return Reply::text("This contributor has nothing on offer right now.");
    }
    let lines = book
        .roles
        .iter()
        .map(|role| {
            let by = match &role.serving {
                Some(serving) if serving.model.is_some() => {
                    let mut text = format!(
                        "{} · {}",
                        serving.framework.as_deref().unwrap_or("?"),
                        serving.model.as_deref().unwrap_or("?")
                    );
                    if let Some(reasoning) = &serving.reasoning {
                        text.push_str(&format!(" ({reasoning})"));
                    }
                    if let Some(tier) = &serving.tier {
                        text.push_str(&format!(" · {tier}"));
                    }
                    if serving.provisioning_required {
                        text.push_str(" · new agent required");
                    }
                    text
                }
                // A published role with nothing able to serve it is worth
                // SAYING rather than omitting: the project can stop waiting, and
                // the contributor probably wants to know it is unfillable.
                _ => "nothing currently qualifies".to_owned(),
            };
            let caps = [
                role.budget_cap_per_engagement
                    .map(|cap| format!("up to {cap} tokens")),
                role.rate_cap.as_ref().map(|rate| format!("{rate}/day")),
                role.count
                    .map(|count| format!("{} of {count} running", role.running_now.unwrap_or(0))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
            format!(
                "  {} — {by}{}",
                role.role,
                if caps.is_empty() {
                    String::new()
                } else {
                    format!("\n      {caps}")
                }
            )
        })
        .collect::<Vec<_>>();
    // Whether YOUR request auto-joins: the most actionable line in the reply.
    let trust = match book.whitelisted {
        Some(true) => Some(
            "This room is whitelisted: a request inside the caps above joins automatically."
                .to_owned(),
        ),
        Some(false) => Some(
            "This room is not whitelisted, so a request waits for the contributor to decide."
                .to_owned(),
        ),
        None => None,
    };
    let mut body = vec!["On offer from this contributor:".to_owned()];
    body.extend(lines);
    if let Some(trust) = trust {
        body.push(String::new());
        body.push(trust);
    }
    body.push(String::new());
    body.push("Ask with:  !request <role> <tokens> [tokens-per-day]".to_owned());
    Reply::text(body.join("\n"))
}

/// The command this text carries, if it carries one — the `bridge-matrix.js`
/// predicate (:7120-7122) reduced to what native receives. Native intake has the
/// plain body and, for text only, the retained product's own rule: a body whose
/// first non-space character is `!` is a command and never agent input. Files and
/// images are never commands (:7122).
pub fn is_command(body: &str, kind: &str) -> bool {
    matches!(kind, "m.text") && body.trim_start().starts_with('!')
}

// ── Dispatch ────────────────────────────────────────────────────────

/// What the observed native host can say for the reads. Each field is the same
/// tri-state TS read: `None` is "the source did not answer", which TS rendered
/// as `unavailable` / `?` / a `Tmux note:` line rather than as an empty result.
#[derive(Debug, Clone, Default)]
pub struct HostObservation {
    pub status: StatusObservation,
    pub agents: AgentsObservation,
    /// `Some(items)` when tmux answered, `None` when it did not — with
    /// `installed` deciding which of TS's two first answers applies (:803-812).
    pub sessions: Option<Vec<SessionItem>>,
    pub tmux_installed: bool,
    pub offer: OfferBook,
}

/// The outcome of running one parsed line. `Unrenderable` is a command native
/// parsed and authorized but has no renderer for — the tier-2/3 terminal verbs,
/// and `!request`, which TS dispatches as a project-side act rather than a local
/// answer. The caller must NOT invent text for it: the retained product's words
/// came from a handler that does not exist here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatched {
    Answer(Reply),
    /// `!request` (board #79): parsed and authorized; the caller submits the
    /// request into the engagement intake, then renders `request_reply` with
    /// the outcome. TS dispatches it the same two-step way (`:365`).
    Request(Vec<String>),
    Unrenderable,
}

/// `handleCommand` (:322-390): no-command, the tier-0-only refusal, the ACL
/// decision, then the switch. The order is the retained product's and is load
/// bearing — a refusal must not be reachable by a command that would have been
/// refused anyway, and an ACL denial must precede any mutation.
pub fn dispatch(
    text: &str,
    sender_mxid: &str,
    acl: &Acl,
    tier0_only: bool,
    observed: &HostObservation,
) -> Dispatched {
    let Some(parsed) = parse(text) else {
        return Dispatched::Answer(no_command_reply());
    };
    let command = parsed.command.as_str();
    let tier = classify(command);
    if tier0_only && tier > 0 {
        return Dispatched::Answer(tier0_only_refusal(command));
    }
    match acl.authorize(sender_mxid, tier) {
        Ok(_grant) => {}
        Err(reason) => return Dispatched::Answer(acl_refusal(command, reason)),
    }
    match command {
        "!help" => Dispatched::Answer(help()),
        "!offer" => Dispatched::Answer(offer(&observed.offer)),
        "!status" => Dispatched::Answer(status(&observed.status)),
        "!agents" => Dispatched::Answer(agents(
            &observed.agents,
            parsed.args.first().is_some_and(|arg| arg == "all"),
        )),
        "!sessions" => Dispatched::Answer(match &observed.sessions {
            Some(items) => sessions(items),
            None => sessions_unavailable(observed.tmux_installed),
        }),
        // `!request` is a project-side act: the caller submits into the
        // engagement intake, then renders the reply from the outcome. The rest
        // are parsed-and-authorized verbs native has no renderer for.
        "!request" => Dispatched::Request(parsed.args),
        "!groups" | "!group" | "!agent" | "!mcp" | "!bridge" | "!mkgroup" | "!bindroom"
        | "!addmember" | "!rmember" | "!joingroup" | "!dm" | "!identity" | "!spy" | "!rmgroup"
        | "!agentctl" | "!ctl" => Dispatched::Unrenderable,
        _ => Dispatched::Answer(unknown_command(command)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The TS table itself, so a drifted tier is caught: `!help`/`!request`/
    /// `!offer` are public, the reads are tier 1, the admin verbs tier 2, the
    /// terminal ones tier 3 (:30-59).
    #[test]
    fn native_bot_command_tiers() {
        for (command, tier) in [
            ("!help", 0),
            ("!request", 0),
            ("!offer", 0),
            ("!status", 1),
            ("!agents", 1),
            ("!sessions", 1),
            ("!bridge", 1),
            ("!mkgroup", 2),
            ("!bindroom", 2),
            ("!identity", 2),
            ("!spy", 3),
            ("!agentctl", 3),
            ("!ctl", 3),
        ] {
            assert_eq!(classify(command), tier, "{command}");
        }
        // `classifyCommand` defaults an UNKNOWN command to operator tier (:61-63).
        assert_eq!(classify("!nonsense"), 1);
    }

    /// `is_operator` gates the `/thread` directive (TS `msg.trustLevel ===
    /// 'operator'`, backend-v2.js:2265, derived from `MATRIX_OPERATOR_MXIDS`
    /// alone): an admin who is not an operator is NOT trusted for it.
    #[test]
    fn native_thread_directive_operator_gate() {
        let acl = Acl::new(["@alex:test".to_owned()], ["@admin:test".to_owned()], false);
        assert!(acl.is_operator("@alex:test"));
        assert!(!acl.is_operator("@admin:test"));
        assert!(!acl.is_operator("@mallory:test"));
        assert!(!acl.is_operator(""));
    }

    /// `parse` (:306-310): trimming, the lowercased first token, arguments.
    #[test]
    fn native_bot_command_parse() {
        assert_eq!(parse("not a command"), None);
        assert_eq!(parse("  hi  "), None);
        assert_eq!(
            parse("  !AGENTS   all  "),
            Some(Parsed {
                command: "!agents".into(),
                args: vec!["all".into()],
            })
        );
        assert_eq!(
            parse("!help"),
            Some(Parsed {
                command: "!help".into(),
                args: vec![],
            })
        );
    }

    /// The ACL fail-closed ruling (:84-101): tier 0 is never refused, the lists
    /// decide the rest, and an unconfigured ACL refuses with its OWN reason —
    /// unless the explicit permissive opt-in is set.
    #[test]
    fn native_bot_command_acl() {
        let unconfigured = Acl::default();
        assert_eq!(unconfigured.authorize("@any:example.test", 0), Ok("public"));
        assert_eq!(
            unconfigured.authorize("@any:example.test", 1),
            Err("acl_unconfigured")
        );
        assert_eq!(
            unconfigured.authorize("@any:example.test", 3),
            Err("acl_unconfigured")
        );
        let permissive = Acl::new(Vec::new(), Vec::new(), true);
        assert_eq!(permissive.authorize("@any:example.test", 1), Ok("no_acl"));

        let acl = Acl::new(
            vec!["@op:example.test".into()],
            vec!["@admin:example.test".into()],
            false,
        );
        assert_eq!(acl.authorize("@op:example.test", 2), Ok("operator"));
        assert_eq!(acl.authorize("@op:example.test", 3), Err("admin_required"));
        assert_eq!(acl.authorize("@admin:example.test", 3), Ok("admin"));
        assert_eq!(
            acl.authorize("@human:example.test", 1),
            Err("operator_required")
        );
    }

    /// The refusals and the dispatch fallbacks, word for word (:324-326, :388,
    /// :338-345, :352-361).
    #[test]
    fn native_bot_command_refusals() {
        assert_eq!(
            no_command_reply().plain,
            "Send !help for available commands."
        );
        assert_eq!(
            unknown_command("!frobnicate").plain,
            "Unknown command: !frobnicate\nSend !help for available commands."
        );
        assert!(unknown_command("!x").html.is_none());
        assert_eq!(
            tier0_only_refusal("!agents").plain,
            "!agents is unavailable: this Hagency is running without a Matrix bot, so only \
             public commands (!help, !offer, !request) work here. Configure \
             MATRIX_BOT_PASSWORD to enable the rest."
        );
        assert_eq!(
            acl_refusal("!agents", "acl_unconfigured").plain,
            "Access denied: !agents is privileged and no operator ACL is configured on this \
             Hagency. Set MATRIX_OPERATOR_MXIDS (or MATRIX_ADMIN_MXIDS) to the humans who may \
             run it."
        );
        assert_eq!(
            acl_refusal("!spy", "admin_required").plain,
            "Access denied: !spy requires admin privileges."
        );
        assert_eq!(
            acl_refusal("!agents", "operator_required").plain,
            "Access denied: !agents requires operator privileges."
        );
    }

    /// `!help` (:631-689): the exact plain body and html the retained product
    /// printed, and the two-part content `reply` builds.
    #[test]
    fn native_bot_command_help() {
        let help = help();
        assert!(help.plain.starts_with("=== Agent Bridge Bot Commands ==="));
        assert!(
            help.plain
                .contains("  !spy <agent1> <agent2>         — Join an agent DM room to watch")
        );
        assert!(
            help.plain.contains(
                "  !request <role> <tokens> [per-day]  — ask this contributor for an agent"
            )
        );
        assert!(help.plain.ends_with("— Join an agent DM room to watch"));
        assert!(
            help.html
                .as_deref()
                .unwrap()
                .starts_with("<h3>Agent Bridge Bot Commands</h3><b>System:</b><br>")
        );
        assert!(
            help.html
                .as_deref()
                .unwrap()
                .contains("<code>!spy &lt;agent1&gt; &lt;agent2&gt;</code>")
        );
        let content = help.content();
        assert_eq!(content["msgtype"], "m.text");
        assert_eq!(content["body"], help.plain);
        assert_eq!(content["format"], "org.matrix.custom.html");
        assert_eq!(content["formatted_body"], help.html.clone().unwrap());
        // A plain-only reply carries no formatting keys at all (`reply`, :397-404).
        assert_eq!(
            unknown_command("!x").content(),
            json!({"msgtype": "m.text", "body": "Unknown command: !x\nSend !help for available commands."})
        );
    }

    /// `!status` (:694-732), including the tmux-absent shape TS produced.
    #[test]
    fn native_bot_command_status() {
        let reply = status(&StatusObservation {
            agents: 3,
            groups: 2,
            sessions: Some(5),
            tmux_note: None,
        });
        assert_eq!(
            reply.plain,
            "=== System Status ===\nAgents: 3\nGroups: 2\nTmux sessions: 5\nBridge: running"
        );
        assert_eq!(
            reply.html.as_deref().unwrap(),
            "<b>System Status</b><br>Agents: <b>3</b><br>Groups: <b>2</b><br>Tmux sessions: <b>5</b><br>Bridge: <b>running</b>"
        );
        let unavailable = status(&StatusObservation {
            agents: 0,
            groups: 0,
            sessions: None,
            tmux_note: Some("tmux server not available (status may be stale)".into()),
        });
        assert_eq!(
            unavailable.plain,
            "=== System Status ===\nAgents: 0\nGroups: 0\nTmux sessions: unavailable\nBridge: running\nTmux note: tmux server not available (status may be stale)"
        );
    }

    /// `!agents` (:733-801): the online filter, the unknown-session icon, the
    /// identity suffix and the `!agents all` footer.
    #[test]
    fn native_bot_command_agents() {
        let observed = AgentsObservation {
            agents: vec![
                AgentItem {
                    name: "alpha".into(),
                    identity: Some("the planner".into()),
                },
                AgentItem {
                    name: "beta".into(),
                    identity: None,
                },
            ],
            sessions: Some(BTreeSet::from(["alpha".to_owned()])),
            tmux_note: None,
        };
        let online = agents(&observed, false);
        assert_eq!(
            online.plain,
            "=== Online Agents ===\n● alpha — the planner\n\nUse !agents all to see all agents including offline."
        );
        assert!(online.html.as_deref().unwrap().contains(
            "<span style=\"color:#69db7c\">●</span> <b>alpha</b> — <i>the planner</i><br>"
        ));
        let all = agents(&observed, true);
        assert_eq!(
            all.plain,
            "=== All Agents ===\n● alpha — the planner\n○ beta"
        );
        assert!(
            all.html
                .as_deref()
                .unwrap()
                .contains("<span style=\"color:#888\">○</span> <b>beta</b><br>")
        );
        // No sessions observed is "unknown", not "offline": `?` and nothing filtered.
        let unknown = agents(
            &AgentsObservation {
                agents: vec![AgentItem {
                    name: "gamma".into(),
                    identity: None,
                }],
                sessions: None,
                tmux_note: Some("tmux binary not found on bridge host".into()),
            },
            false,
        );
        assert_eq!(
            unknown.plain,
            "=== Online Agents ===\n? gamma\n\nTmux note: tmux binary not found on bridge host"
        );
        assert_eq!(
            agents(&AgentsObservation::default(), false).plain,
            "No known agents yet."
        );
    }

    /// `!sessions` (:802-836): the no-binary branch, the no-sessions branch and
    /// the populated table.
    #[test]
    fn native_bot_command_sessions() {
        assert_eq!(
            sessions_unavailable(false).plain,
            "tmux is not installed on bridge host."
        );
        assert_eq!(
            sessions_unavailable(true).plain,
            "No tmux sessions found (tmux server not running)."
        );
        assert_eq!(sessions(&[]).plain, "No tmux sessions found.");
        let reply = sessions(&[
            SessionItem {
                name: "alpha".into(),
                process: "node".into(),
            },
            SessionItem {
                name: "beta".into(),
                process: "-".into(),
            },
        ]);
        assert_eq!(
            reply.plain,
            "=== Tmux Sessions ===\n  alpha: node\n  beta: -\n\nTotal: 2"
        );
        assert_eq!(
            reply.html.as_deref().unwrap(),
            "<b>Tmux Sessions</b><br><br><code>alpha</code>: <b>node</b><br><code>beta</code>: <b>-</b><br><br>Total: <b>2</b>"
        );
    }

    /// `!request` reply half (board #79; TS `lib/bot-commands.js:525-629`):
    /// the usage text, the malformed-token refusal (before any backend), the
    /// auto-joined transparency line, the attach note without the provider's
    /// remedy, and the pending-why routing.
    #[test]
    fn native_bot_command_request_reply() {
        let usage = "Usage: !request <role> <tokens> [tokens-per-day]\n\
                     Example: !request architect 400000 20000";
        // Usage: role or tokens missing (:528-531).
        assert_eq!(request_reply(&[], None).plain, usage);
        assert_eq!(request_reply(&["coding".into()], None).plain, usage);
        // Malformed token: names the offending word, never reaches the
        // backend (:533-536).
        assert_eq!(
            request_reply(&["coding".into(), "four-hundred-thousand".into()], None).plain,
            "Not a token amount: four-hundred-thousand"
        );
        // `_,`-separated digits parse (:533).
        assert_eq!(
            request_reply(&["coding".into(), "400_000".into()], None).plain,
            "Request failed: no engagement returned."
        );
        // No outcome at all (:546).
        assert_eq!(
            request_reply(&["coding".into(), "400000".into()], None).plain,
            "Request failed: no engagement returned."
        );
        // Backend refusal (:543-544).
        assert_eq!(
            request_reply(
                &["coding".into(), "400000".into()],
                Some(&RequestOutcome {
                    error: Some("over ceiling".into()),
                    ..RequestOutcome::default()
                })
            )
            .plain,
            "Request refused: over ceiling"
        );
        // Auto-joined with the transparency config (:596-605); `?` for the
        // unknown framework exactly as TS.
        assert_eq!(
            request_reply(
                &["coding".into(), "400000".into()],
                Some(&RequestOutcome {
                    engagement: Some(RequestEngagement {
                        role: "coding".into(),
                        requested_tokens: 400_000,
                        allocated_tokens: 400_000,
                        agent: "claude-agent".into(),
                        auto_joined: true,
                        route: None,
                    }),
                    serving: Some(OfferServing {
                        framework: Some("claude".into()),
                        model: Some("claude-opus-5".into()),
                        reasoning: Some("high".into()),
                        tier: Some("strong".into()),
                        ..OfferServing::default()
                    }),
                    ..RequestOutcome::default()
                })
            )
            .plain,
            "Joined automatically as coding — 400000 tokens, served by claude-agent \
             running claude · claude-opus-5 (high) · strong."
        );
        // Serving present but no model: degrades to the agent alone, no
        // fabricated configuration (:607-612 via `s?.model` guard).
        assert_eq!(
            request_reply(
                &["coding".into(), "400000".into()],
                Some(&RequestOutcome {
                    engagement: Some(RequestEngagement {
                        role: "coding".into(),
                        requested_tokens: 400_000,
                        allocated_tokens: 400_000,
                        agent: "claude-agent".into(),
                        auto_joined: true,
                        route: None,
                    }),
                    serving: Some(OfferServing::default()),
                    ..RequestOutcome::default()
                })
            )
            .plain,
            "Joined automatically as coding — 400000 tokens, served by claude-agent."
        );
        // The attach failure note carries no HAGENCY_* remedy (:606-618).
        let attach = request_reply(
            &["coding".into(), "400000".into()],
            Some(&RequestOutcome {
                engagement: Some(RequestEngagement {
                    role: "coding".into(),
                    requested_tokens: 400_000,
                    allocated_tokens: 400_000,
                    agent: "claude-agent".into(),
                    auto_joined: true,
                    route: None,
                }),
                serving: Some(OfferServing::default()),
                attach_failed: true,
                ..RequestOutcome::default()
            }),
        );
        assert!(attach.plain.contains("could not be attached yet"));
        assert!(!attach.plain.contains("HAGENCY_"));
        assert!(!attach.plain.contains("MXID"));
        // Pending with each WHY word, and the default (:619-628).
        for (route, why) in [
            (
                Some("notWhitelisted"),
                "this room is not on the contributor's whitelist",
            ),
            (
                Some("overOffer"),
                "the amount is above what they have published",
            ),
            (
                Some("overCeiling"),
                "the amount is above what the serving agent has left",
            ),
            (None, "it needs a decision"),
        ] {
            assert_eq!(
                request_reply(
                    &["coding".into(), "400000".into()],
                    Some(&RequestOutcome {
                        engagement: Some(RequestEngagement {
                            role: "coding".into(),
                            requested_tokens: 400_000,
                            allocated_tokens: 0,
                            agent: String::new(),
                            auto_joined: false,
                            route: route.map(str::to_owned),
                        }),
                        ..RequestOutcome::default()
                    })
                )
                .plain,
                format!("Requested coding for 400000 tokens — awaiting a decision, because {why}.")
            );
        }
    }

    /// `!offer` (:469-524): the empty state, the error state, the serving line
    /// with its caps, and the tri-state trust line.
    #[test]
    fn native_bot_command_offer() {
        assert_eq!(
            offer(&OfferBook::default()).plain,
            "This contributor has nothing on offer right now."
        );
        assert_eq!(
            offer(&OfferBook {
                error: Some("boom".into()),
                ..OfferBook::default()
            })
            .plain,
            "Cannot read the offer book: boom"
        );
        let reply = offer(&OfferBook {
            error: None,
            roles: vec![
                OfferRole {
                    role: "architect".into(),
                    serving: Some(OfferServing {
                        framework: Some("codex".into()),
                        model: Some("gpt-5".into()),
                        reasoning: Some("high".into()),
                        tier: Some("pro".into()),
                        provisioning_required: true,
                    }),
                    budget_cap_per_engagement: Some(400_000),
                    rate_cap: Some("20000".into()),
                    count: Some(3),
                    running_now: Some(1),
                },
                OfferRole {
                    role: "reviewer".into(),
                    serving: None,
                    budget_cap_per_engagement: None,
                    rate_cap: None,
                    count: None,
                    running_now: None,
                },
            ],
            whitelisted: Some(true),
        });
        assert_eq!(
            reply.plain,
            "On offer from this contributor:\n  architect — codex · gpt-5 (high) · pro · new agent required\n      up to 400000 tokens, 20000/day, 1 of 3 running\n  reviewer — nothing currently qualifies\n\nThis room is whitelisted: a request inside the caps above joins automatically.\n\nAsk with:  !request <role> <tokens> [tokens-per-day]"
        );
        assert!(reply.html.is_none());
        // `null` whitelist is "no room identified", NOT a refusal: no trust line.
        let no_room = offer(&OfferBook {
            roles: vec![OfferRole {
                role: "reviewer".into(),
                ..OfferRole::default()
            }],
            whitelisted: None,
            ..OfferBook::default()
        });
        assert!(!no_room.plain.contains("whitelisted"));
        assert!(
            no_room
                .plain
                .contains("reviewer — nothing currently qualifies")
        );
    }

    /// The command predicate that keeps a `!` line out of agent input
    /// (`bridge-matrix.js:7120-7122`).
    #[test]
    fn native_bot_command_predicate() {
        assert!(is_command("!help", "m.text"));
        assert!(is_command("  !status  ", "m.text"));
        assert!(!is_command("hello !help", "m.text"));
        assert!(!is_command("!help", "m.image"));
        assert!(!is_command("!help", "m.file"));
        assert!(!is_command("", "m.text"));
    }

    /// `handleCommand` (:322-390) end to end: the no-command reply, tier-0
    /// commands answering with no ACL configured at all, the ACL denial of a
    /// tier-1 read on an unconfigured deployment, the unknown-command reply, and
    /// a parsed-but-unrenderable verb producing no invented words.
    #[test]
    fn native_bot_command_dispatch() {
        let empty = Acl::new(Vec::new(), Vec::new(), false);
        let observed = HostObservation::default();
        // Not a command at all (:324-326).
        assert_eq!(
            dispatch("hello", "@a:example.test", &empty, false, &observed),
            Dispatched::Answer(no_command_reply())
        );
        // Tier 0 answers even with an empty (unconfigured) ACL: public.
        assert_eq!(
            dispatch("!help", "@a:example.test", &empty, false, &observed),
            Dispatched::Answer(help())
        );
        assert_eq!(
            dispatch("!offer", "@a:example.test", &empty, false, &observed),
            Dispatched::Answer(offer(&observed.offer))
        );
        // A tier-1 read on a deployment with no ACL configured is refused with
        // its own distinct reason, exactly as TS (:84-101).
        assert_eq!(
            dispatch("!status", "@a:example.test", &empty, false, &observed),
            Dispatched::Answer(acl_refusal("!status", "acl_unconfigured"))
        );
        let operator = Acl::new(vec!["@a:example.test".to_owned()], Vec::new(), false);
        // An unknown command reached by an operator gets the TS words, and a
        // typo is not a free pass: it defaults to tier 1 (:61-63, :388).
        assert_eq!(
            dispatch("!nonsense", "@a:example.test", &operator, false, &observed),
            Dispatched::Answer(unknown_command("!nonsense"))
        );
        for command in ["!bindroom g", "!dm someone"] {
            assert_eq!(
                dispatch(command, "@a:example.test", &operator, false, &observed),
                Dispatched::Unrenderable,
                "{command}"
            );
        }
        // `!request` is parsed, authorized and handed to the caller to submit
        // into the engagement intake (board #79; TS :365).
        assert_eq!(
            dispatch(
                "!request r 1000",
                "@a:example.test",
                &operator,
                false,
                &observed
            ),
            Dispatched::Request(vec!["r".to_owned(), "1000".to_owned()])
        );
        // A bridge running without a bot refuses a privileged command before
        // any mutation (:338-345).
        assert_eq!(
            dispatch("!status", "@a:example.test", &operator, true, &observed),
            Dispatched::Answer(tier0_only_refusal("!status"))
        );
    }
}
