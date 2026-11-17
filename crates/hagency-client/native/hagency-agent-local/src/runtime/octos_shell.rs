//! The owner's local Octos Agent runs commands in the same confined shell as
//! a pooled one: the workspace the owner released for the Agent's files, no
//! network and no private configuration. Each command goes through the
//! owner's tool policy (full trust or a question to the owner) and the
//! current dispatch lease, checked again after the owner's answer.
use super::*;
use hagency_runtime::owned::SharedOctosShell;
use serde_json::json;
use std::collections::VecDeque;

type Buffered = VecDeque<(octos::session::Event, Option<octos::session::Observation>)>;

/// How long the owner has to answer one command.
const APPROVAL_SECS: u64 = 100;
/// The longest a command may run.
const MAX_SHELL_SECS: u64 = 90;
// A command answered at the last moment still runs its longest time inside
// Octos's deadline for the host tool call, with time to send the result.
const _: () = assert!((APPROVAL_SECS + MAX_SHELL_SECS + 10) * 1000
    <= hagency_runtime::octos::session::HOST_TOOL_TIMEOUT_MS);
pub(super) const TOOL: &str = "hagency.shell";

pub(super) fn definition() -> octos::session::HostTool {
    octos::session::HostTool {
        name: TOOL.into(),
        description: "Run a command in this Agent's workspace. Trusted system Apple Command Line Tools may be available read-only; commands cannot read private configuration or use the network. Each command needs the owner's approval.".into(),
        input_schema: json!({"type":"object","properties":{
            "command":{"type":"string"},"timeout_secs":{"type":"integer","minimum":1,"maximum":MAX_SHELL_SECS}},
            "required":["command"],"additionalProperties":false}),
        read_only: false,
    }
}

/// The shell lives only while this turn does: dropping the guard ends it,
/// and a command still running is stopped with its whole process tree.
pub(super) struct Turn(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Turn {
    pub(super) fn start(host: &OctosShellHost, files: &OctosHostFiles) -> (Self, SharedOctosShell) {
        let live = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let shell = SharedOctosShell::for_turn(
            host.guardian.clone(),
            files.workspace.canonical_directory().to_path_buf(),
            live.clone(),
            host.pending.clone(),
        );
        (Self(live), shell)
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    command: String,
    timeout_secs: Option<u64>,
}
fn input(args: &Value) -> std::result::Result<(String, u64), &'static str> {
    let input: Input = serde_json::from_value(args.clone()).map_err(|_| "invalid_shell_request")?;
    let timeout = input.timeout_secs.unwrap_or(30);
    if input.command.is_empty() || input.command.len() > 16_000 || input.command.contains('\0')
        || !(1..=MAX_SHELL_SECS).contains(&timeout)
    {
        return Err("shell_request_refused");
    }
    Ok((input.command, timeout))
}

async fn reply(wire: &mut Octos, call: &str, value: std::result::Result<Value, String>) -> Result<()> {
    wire.host_tool_result(call, value)
        .await
        .map_err(|_| Error::Protocol("Octos shell tool response"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle(
    wire: &mut Octos,
    shell: &SharedOctosShell,
    files: &OctosHostFiles,
    ledger: &mut Ledger,
    owner: &str,
    scope: &Scope,
    execution: &str,
    dispatch: &str,
    call: &octos::session::HostToolCall,
    queue: &ApprovalQueue,
    controls: &mut Option<crate::run_controls::Stream>,
    buffered: &mut Buffered,
    stopping: &mut bool,
    steers: &mut OctosSteers,
) -> Result<()> {
    if *stopping || !wire.host_tool_pending(&call.call_id) {
        return Ok(());
    }
    let (command, timeout) = match input(&call.args) {
        Ok(input) => input,
        Err(code) => return reply(wire, &call.call_id, Err(code.into())).await,
    };
    let gate = files.gate.clone();
    for checked in [false, true] {
        // The dispatch must still hold, before the owner is asked and again
        // after the answer, immediately before the command starts.
        let permitted = tokio::time::timeout(Duration::from_secs(5), gate.authorize(dispatch, execution));
        tokio::pin!(permitted);
        match octos_wait_tool(wire, permitted.as_mut(), controls, buffered, stopping, steers, &call.call_id).await? {
            None => return Ok(()),
            Some(Ok(true)) => {}
            _ => return reply(wire, &call.call_id, Err("server_scope_unavailable".into())).await,
        }
        if checked {
            break;
        }
        let decision = permission_within(
            ledger,
            owner,
            scope,
            dispatch,
            "octos.shell".into(),
            json!({"command":command,"timeout_secs":timeout}),
            files.workspace.canonical_directory(),
            queue,
            APPROVAL_SECS,
            false,
        );
        tokio::pin!(decision);
        match octos_wait_tool(wire, decision.as_mut(), controls, buffered, stopping, steers, &call.call_id).await? {
            None => return Ok(()),
            Some(claude_turn::ToolAnswer::Allow) => {}
            Some(claude_turn::ToolAnswer::Unanswered) => {
                return reply(wire, &call.call_id, Err("shell_owner_unanswered".into())).await;
            }
            Some(claude_turn::ToolAnswer::Deny) => {
                return reply(wire, &call.call_id, Err("shell_owner_denied".into())).await;
            }
        }
    }
    let run = shell.run(&command, Duration::from_secs(timeout));
    tokio::pin!(run);
    // A stop or a cancelled call drops the command, which stops its tree.
    let Some(result) = octos_wait_tool(wire, run.as_mut(), controls, buffered, stopping, steers, &call.call_id).await? else {
        return Ok(());
    };
    let value = result
        .map_err(str::to_owned)
        .and_then(|result| serde_json::to_value(result).map_err(|_| "shell_output_unavailable".into()));
    reply(wire, &call.call_id, value).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_bounded_before_anyone_is_asked() {
        assert_eq!(input(&json!({"command":"pwd"})), Ok(("pwd".into(), 30)));
        assert_eq!(input(&json!({"command":"ls","timeout_secs":90})), Ok(("ls".into(), 90)));
        for refused in [
            json!({"command":""}),
            json!({"command":"ls","timeout_secs":0}),
            json!({"command":"ls","timeout_secs":91}),
            json!({"command":"a\u{0}b"}),
            json!({"command":"x".repeat(16_001)}),
        ] {
            assert_eq!(input(&refused), Err("shell_request_refused"), "{refused}");
        }
        assert_eq!(input(&json!({"command":"ls","cwd":"/"})), Err("invalid_shell_request"));
        assert_eq!(input(&json!({"cmd":"ls"})), Err("invalid_shell_request"));
    }

    #[test]
    fn the_tool_offers_no_longer_command_than_the_deadline_allows() {
        let tool = definition();
        assert_eq!(tool.name, TOOL);
        assert_eq!(tool.input_schema["properties"]["timeout_secs"]["maximum"], MAX_SHELL_SECS);
        assert!(!tool.read_only);
    }
}
