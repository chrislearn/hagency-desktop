# ADR 0014: A standing role for a local Agent in a room

- Date: 2026-11-18
- Status: Accepted; implemented for local Agents. Pooled Agents are not covered.
- Scope: Hagency Desktop and its embedded Hagency Client. No server change.
- Builds on the host-kept timers of
  [ADR 0013](0013-spontaneous-agent-messages.md).

## Context

A project is an Agent space with several rooms, and a person gives an Agent a
role in a room: reviewer, tester, the one who writes the design notes. The
Agent forgot that role with the next message.

- **Server.** In a room of an Agent space every top-level message is the root
  of its own thread. Only the owner's direct chat uses the room as the root.
- **Desktop.** A local Agent keeps one native session per owner, Agent,
  binding, room, requester and thread. A new thread is a new session.
- **Prompt.** A local Agent's request is not preceded by earlier room
  messages. The model can read them with `read_room_history`, and nothing
  tells it that there is something to look for.

Each of the three is intended. Threads keep the tasks of a room apart and let
them run side by side, and a session per requester keeps one member's
conversation and trust out of another's. What was missing is something that
holds across the threads of a room.

## Decision

The host keeps a standing role per room and puts it before every request of
that room.

**Tools.** A local Agent gets two more room tools:

- `set_room_role` with one field, `role`, of at most 2000 bytes. It replaces
  the role saved before. Empty text removes it.
- `get_room_role` with no field.

The model names neither the room nor a person. The role is saved for the room
of the run that called the tool and for the person who asked for that run.

**Store.** One sealed file per room and person, in the folder of the Agent's
device, beside the timers. The file's name is a digest; the text is not on
disk in the clear.

**Whose role reaches whom.**

- What the Agent's owner saved applies to every conversation in the room,
  whoever asks.
- What another member saved applies to that member's own conversations in the
  room, and to nobody else's.
- Where both exist, both are given, the owner's first, and the text says that
  the owner's comes first.

A member cannot replace or remove the owner's role. The tool's result says
whom a saved role applies to, so the Agent can tell a member that others will
not see it.

**Why not one role per room for everybody.** A role is placed before a request
as an instruction. An owner can give a local Agent full trust for the owner's
own messages. If a member's text reached the owner's conversation, the member
would be giving instructions that run without anybody being asked.

**Prompt.** Before every request of a local Agent the host puts the roles that
reach it, with a sentence that says to keep to them and to save a change. It
does so on every turn, not only at the start of a session: a retained session
may be older than the role, and a native session may have dropped it when it
compacted. Where no role is saved and each message of the room starts its own
conversation, one sentence says that a role must be saved to be kept. In the
owner's direct chat nothing is added.

**No approval.** Saving a role runs nothing, sends nothing and reads nothing of
the room, so nobody is asked for it, as for a timer. One exception, also as
for a timer: in a turn that holds a message from another Claude Code session
the owner is asked before a role is saved, because it would later be handed to
every conversation of the room.

## Limits

- **Local Agents only.** A pooled Agent has no role tools. Its first request in
  a new conversation is preceded by a page of recent room messages, as data.
- **The model has to save it.** A role is kept when the model calls
  `set_room_role`. The tool's description, the sentence before the request
  and, for Claude Code, the room note ask for it. Nothing in the host reads a
  message and decides that it gives a role.
- **Other Agents do not see it.** A role is known to the Agent it was given
  to. Another Agent in the room learns it only from what is said in the room.
- **A slash command.** The owner's own message that starts with `/` reaches a
  local Claude Agent as written, without the role.
- **No page in the app** shows or edits a role. `get_room_role` reads it.

## Evidence

Live on the production server, 2026-11-18, in an encrypted room of an Agent
space, with a local Codex Agent and a local Claude Code Agent of the same
owner:

- A top-level message gave the Codex Agent the role of test engineer with a
  prefix for every answer. The Agent called `set_room_role`
  (`agent_room_role_saved`, for everyone), and nobody was asked.
- The next top-level message, a new conversation, asked what it does in the
  room. The answer began with the prefix and named the role.
- The same two messages to the Claude Code Agent, with the role of reviewer:
  saved, and the answer in the new conversation kept it. The two Agents hold
  different roles in the same room.
- A message that ended the role removed it for each Agent
  (`agent_room_role_removed`).

Not run live: another member's own instructions, and the owner's role as it
reaches another member's conversation. Both are covered by the unit tests of
`owned_runtime/roles.rs`.
