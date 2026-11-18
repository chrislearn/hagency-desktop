//! A standing role for one room (`hagency_runtime::room_tools::ROOM_ROLE_TOOLS`).
//!
//! In a room of an Agent space the server makes every top-level message the
//! root of its own thread, and a local Agent keeps one native session per
//! person and thread. A role given in one message was therefore unknown to
//! the answer to the next one. The host keeps the role instead: sealed in the
//! folder of the Agent's device, and placed before every request of that room.
//!
//! Whose role reaches whom. What the Agent's owner saved applies to every
//! conversation in the room. What another member saved applies to that
//! member's own conversations only: a member's words never become
//! instructions in a conversation that runs with somebody else's trust.
use super::messages::roles::{self, Role};
use super::*;
use hagency_runtime::room_tools::RoomRoleOperation;
use serde_json::json;

const EVERYONE: &str = "every conversation in this room";
const OWN: &str = "this person's conversations in this room";

fn reach(by: &str, owner: &str) -> &'static str {
    if by == owner { EVERYONE } else { OWN }
}
fn shown(role: &Role, owner: &str) -> serde_json::Value {
    json!({
        "role": role.text,
        "given_by": role.by,
        "applies_to": reach(&role.by, owner),
        "saved_unix_seconds": role.set,
    })
}
/// The roles that reach a request of `dispatch`: the owner's, then the
/// asking person's own.
fn reaching(root: &std::path::Path, dispatch: &inbox::Dispatch, owner: &str) -> Vec<Role> {
    if !roles::any(root) {
        return Vec::new();
    }
    let asking = dispatch.requester_mxid.as_str();
    let mut found: Vec<Role> = roles::get(root, &dispatch.room_id, owner).into_iter().collect();
    if asking != owner {
        found.extend(roles::get(root, &dispatch.room_id, asking));
    }
    found
}

/// One call of a role tool in the run of `dispatch`. The role is saved for
/// that run's room and for the person who asked for that run; the model names
/// neither. The reply is the tool's JSON result.
pub(super) fn tool(
    root: &std::path::Path,
    dispatch: &inbox::Dispatch,
    owner: &str,
    operation: &RoomRoleOperation,
) -> String {
    let (room, by) = (dispatch.room_id.as_str(), dispatch.requester_mxid.as_str());
    match operation {
        RoomRoleOperation::Invalid { tool, reason } => {
            // The reason is one of the host's own sentences, never the model's words.
            tracing::info!(code = "agent_room_role_tool_invalid", tool, reason,
                "A room role tool was called with arguments that cannot be used");
            json!({ "error": "invalid_arguments", "hint": reason }).to_string()
        }
        RoomRoleOperation::Set { role } if role.is_empty() => {
            let had = roles::get(root, room, by).is_some();
            roles::remove(root, room, by);
            tracing::info!(code = "agent_room_role_removed", had, for_everyone = by == owner,
                "A local Agent's standing role for a room was removed");
            json!({ "status": if had { "removed" } else { "none_was_saved" }, "applied_to": reach(by, owner) })
                .to_string()
        }
        RoomRoleOperation::Set { role } => {
            let saved = Role { version: 1, room: room.into(), by: by.into(), set: now(), text: role.clone() };
            if roles::save(root, &saved).is_err() {
                return json!({ "error": "role_store_unavailable" }).to_string();
            }
            tracing::info!(code = "agent_room_role_saved", bytes = role.len(), for_everyone = by == owner,
                "A local Agent saved a standing role for a room");
            let mut result = json!({ "status": "saved", "applies_to": reach(by, owner) });
            if by != owner {
                result["note"] = json!("Only the owner of this Agent can give a role that applies to \
                    everyone in the room. Say so if the person expects others to see this role.");
            }
            result.to_string()
        }
        RoomRoleOperation::Get {} => {
            let found: Vec<_> = reaching(root, dispatch, owner).iter().map(|role| shown(role, owner)).collect();
            json!({ "roles": found }).to_string()
        }
    }
}

/// What the host puts before a request of `dispatch`: the roles that reach
/// it. Where none is saved and each message of the room starts its own
/// conversation, one sentence says how to keep one. Empty otherwise.
pub(super) fn standing(root: &std::path::Path, dispatch: &inbox::Dispatch, owner: &str) -> String {
    let saved = saved(root, dispatch, owner);
    if saved.is_empty() && dispatch.thread_root != dispatch.room_id {
        return "No standing role is saved for you in this room. If the current request \
            gives you a role or standing instructions for this room, save them with set_room_role \
            before you answer: a later conversation in this room starts without this one.\n\n"
            .into();
    }
    saved
}
/// The roles that reach the conversation of `dispatch`, as the host puts
/// them before a request and before the note of a timer's turn. Empty where
/// none is saved.
pub(super) fn saved(root: &std::path::Path, dispatch: &inbox::Dispatch, owner: &str) -> String {
    let found = reaching(root, dispatch, owner);
    let mut text = String::new();
    for role in &found {
        text.push_str(if role.by == owner {
            "Your standing role in this room. The owner of this Agent gave it, you saved it with \
             set_room_role, and it applies to every conversation in this room:\n"
        } else {
            "Standing instructions the person asking now gave you for their own conversations in \
             this room, saved with set_room_role. Where they differ from the owner's role, the \
             owner's role comes first:\n"
        });
        text.push_str(&role.text);
        text.push_str("\n\n");
    }
    if !found.is_empty() {
        text.push_str("Keep to this in your answer. If the current request changes or ends it, \
            save the new text with set_room_role.\n\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: &str = "@owner:example.test";
    const GUEST: &str = "@guest:example.test";
    const OTHER: &str = "@other:example.test";
    fn dispatch(room: &str, thread: &str, requester: &str) -> inbox::Dispatch {
        serde_json::from_value(json!({
            "id":"dsp_one","bindingId":"bnd_one","agentId":"agt_one","eventId":"$event:example.test",
            "roomId":room,"requesterMxid":requester,"threadRoot":thread,"body":"hello",
            "state":"running","bindingGeneration":1
        }))
        .unwrap()
    }
    fn set(text: &str) -> RoomRoleOperation {
        RoomRoleOperation::Set { role: text.into() }
    }
    fn result(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }
    #[test]
    fn the_owners_role_reaches_every_conversation_and_a_members_only_their_own() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        const ROOM: &str = "!project:example.test";
        let owner_asks = dispatch(ROOM, "$one:example.test", OWNER);
        // Nothing saved: a room whose messages each start a conversation is
        // told how to keep a role; the owner's direct chat is told nothing.
        let none = standing(root, &owner_asks, OWNER);
        assert!(none.starts_with("No standing role is saved") && none.contains("set_room_role"));
        assert!(standing(root, &dispatch(ROOM, ROOM, OWNER), OWNER).is_empty());
        // A look saved nothing and made no key.
        assert!(!roles::any(root));
        assert_eq!(result(&tool(root, &owner_asks, OWNER, &RoomRoleOperation::Get {}))["roles"], json!([]));

        let first = result(&tool(root, &owner_asks, OWNER, &set("You are the reviewer of this project.")));
        assert_eq!((first["status"].as_str(), first["applies_to"].as_str()), (Some("saved"), Some(EVERYONE)));
        assert!(first.get("note").is_none());
        // The next top-level message, by the owner and by anybody else.
        for asking in [OWNER, GUEST, OTHER] {
            let next = standing(root, &dispatch(ROOM, "$two:example.test", asking), OWNER);
            assert!(next.contains("You are the reviewer of this project."), "{asking}");
            assert!(next.contains("every conversation in this room") && next.ends_with("\n\n"));
        }
        // Another room of the same Agent has no role.
        assert!(!standing(root, &dispatch("!elsewhere:example.test", "$t:example.test", OWNER), OWNER)
            .contains("reviewer"));

        // A member's instructions stay in that member's conversations.
        let guest_asks = dispatch(ROOM, "$three:example.test", GUEST);
        let own = result(&tool(root, &guest_asks, OWNER, &set("Answer me in French.")));
        assert_eq!(own["applies_to"], OWN);
        assert!(own["note"].as_str().unwrap().contains("Only the owner"));
        let for_guest = standing(root, &dispatch(ROOM, "$four:example.test", GUEST), OWNER);
        assert!(for_guest.contains("reviewer") && for_guest.contains("Answer me in French."));
        assert!(for_guest.find("reviewer").unwrap() < for_guest.find("French").unwrap());
        for asking in [OWNER, OTHER] {
            assert!(!standing(root, &dispatch(ROOM, "$five:example.test", asking), OWNER).contains("French"));
        }
        let listed = result(&tool(root, &guest_asks, OWNER, &RoomRoleOperation::Get {}));
        assert_eq!(listed["roles"].as_array().unwrap().len(), 2);
        assert_eq!(listed["roles"][0]["given_by"], OWNER);
        assert_eq!(listed["roles"][1]["applies_to"], OWN);
        assert_eq!(result(&tool(root, &owner_asks, OWNER, &RoomRoleOperation::Get {}))["roles"].as_array().unwrap().len(), 1);

        // A member cannot replace or remove the owner's role.
        assert_eq!(result(&tool(root, &guest_asks, OWNER, &set("")))["status"], "removed");
        assert_eq!(result(&tool(root, &guest_asks, OWNER, &set("")))["status"], "none_was_saved");
        assert!(standing(root, &guest_asks, OWNER).contains("reviewer"));
        // The owner replaces it, then removes it.
        tool(root, &owner_asks, OWNER, &set("You are the tester."));
        let replaced = standing(root, &guest_asks, OWNER);
        assert!(replaced.contains("You are the tester.") && !replaced.contains("reviewer"));
        assert_eq!(result(&tool(root, &owner_asks, OWNER, &set("")))["status"], "removed");
        assert!(standing(root, &guest_asks, OWNER).starts_with("No standing role is saved"));
        // A timer's turn gets the role and never the sentence about saving one.
        assert!(saved(root, &guest_asks, OWNER).is_empty());
        tool(root, &owner_asks, OWNER, &set("You are the tester."));
        assert_eq!(saved(root, &guest_asks, OWNER), standing(root, &guest_asks, OWNER));
    }
    #[test]
    fn a_saved_role_is_sealed_and_an_unusable_call_saves_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let asks = dispatch("!project:example.test", "$one:example.test", OWNER);
        let refused = result(&tool(root, &asks, OWNER,
            &RoomRoleOperation::Invalid { tool: "set_room_role", reason: "the reason" }));
        assert_eq!((refused["error"].as_str(), refused["hint"].as_str()), (Some("invalid_arguments"), Some("the reason")));
        assert!(!roles::any(root));
        tool(root, &asks, OWNER, &set("the words of the role are not on disk in the clear"));
        let folder = root.join("outgoing-messages").join("roles");
        let files: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 1);
        let bytes = std::fs::read(&files[0]).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("the words of the role"));
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.contains("project") && !name.contains("owner") && name.ends_with(".sealed"));
        // Saving again replaces the one file.
        tool(root, &asks, OWNER, &set("second"));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        assert!(standing(root, &asks, OWNER).contains("second"));
    }
}
