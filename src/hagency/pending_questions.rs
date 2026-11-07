//! Questions pooled Agents ask a room during a run (Codex `request_user_input`),
//! shown as cards under the message that started the run.
//!
//! The cards come from the room itself. The Agent's device posts each card as
//! a message with a custom msgtype, which the timeline does not draw as a
//! message (other clients show its text body), and edits it once it is
//! answered or expired. Any member of the room may answer: the answer is a
//! custom room event, encrypted like any other in an encrypted room, which the
//! server never treats as a chat message, so it never queues behind the run.
//! The Agent takes the first valid answer from a current member. Nothing here
//! decides who may answer; this module only presents cards and sends answers.
use ::hagency::agent_questions::{self as protocol, AnswerValue, Card, CardState};
use makepad_widgets::*;
use matrix_sdk::{
    Client, Room,
    deserialized_responses::EncryptionInfo,
    ruma::{OwnedRoomId, OwnedUserId, RoomId, events::AnySyncTimelineEvent, serde::Raw},
};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// Cards kept for presentation, newest last. Older ones are dropped.
const MAX_CARDS: usize = 256;

/// What this client did with a card since it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalAnswer {
    None,
    Sending,
    Sent,
    Failed(String),
}

/// One card as posted in a room, with this client's choices in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionCard {
    pub room_id: String,
    /// The card message itself, which answers refer to.
    pub card_event: String,
    pub agent: String,
    pub agent_name: String,
    /// The latest state the Agent published (the original or its edit).
    pub card: Card,
    pub local: LocalAnswer,
    /// Question id to the chosen option label.
    pub choices: BTreeMap<String, String>,
    /// Question id to typed text.
    pub texts: BTreeMap<String, String>,
}
impl QuestionCard {
    pub fn id(&self) -> &str {
        &self.card.id
    }
    /// Buttons work only while the Agent waits and nothing was sent yet.
    pub fn open(&self, now_ms: u64) -> bool {
        self.card.is_open(now_ms) && matches!(self.local, LocalAnswer::None | LocalAnswer::Failed(_))
    }
    /// The answers chosen so far, in the shape the Agent accepts.
    pub fn answers(&self) -> BTreeMap<String, AnswerValue> {
        let mut answers = BTreeMap::new();
        for q in &self.card.questions {
            if let Some(text) = self.texts.get(&q.id).map(|t| t.trim()).filter(|t| !t.is_empty()) {
                answers.insert(q.id.clone(), AnswerValue::Text(text.to_owned()));
            } else if let Some(label) = self.choices.get(&q.id) {
                answers.insert(q.id.clone(), AnswerValue::Option(label.clone()));
            }
        }
        answers
    }
    /// Every question has a choice or typed text that the card accepts.
    pub fn ready(&self) -> bool {
        let answers = self.answers();
        answers.len() == self.card.questions.len() && self.card.answer_content(&self.card_event, &answers).is_ok()
    }
}

/// Answer content for the room, produced by a click and sent by the app.
#[derive(Clone, Debug)]
pub struct QuestionAnswerRequested {
    pub room_id: String,
    pub card_id: String,
    pub agent: String,
    pub content: Value,
}
/// Posted after the answer reached the homeserver, or failed to.
#[derive(Debug)]
pub struct QuestionAnswerSent {
    pub card_id: String,
    pub result: Result<(), String>,
}
/// Posted whenever cards arrive or change, so open timelines repopulate.
#[derive(Debug, Clone, Copy)]
pub struct PendingQuestionsChanged;

#[derive(Default)]
struct State {
    owner: String,
    cards: Vec<QuestionCard>,
    /// Edits seen before their card, applied when the card arrives.
    early_edits: Vec<(String, String, String, Card)>,
    announced: HashSet<String>,
}
static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Collect cards from this client's own sync. A new client (another account,
/// or a re-login) starts with no cards.
pub fn attach(client: &Client) {
    reset(client.user_id().map(|u| u.to_string()).unwrap_or_default());
    client.add_event_handler(
        |event: Raw<AnySyncTimelineEvent>, room: Room, info: Option<EncryptionInfo>| async move {
            let Ok(value) = event.deserialize_as::<Value>() else { return };
            if value["type"] != "m.room.message" || value["content"]["msgtype"] != protocol::MSGTYPE {
                return;
            }
            let (Some(sender), Some(event_id)) = (value["sender"].as_str(), value["event_id"].as_str()) else {
                return;
            };
            // An encrypted card counts only from its own Megolm sender.
            if !protocol::is_agent(sender) || info.as_ref().is_some_and(|i| i.sender.as_str() != sender) {
                return;
            }
            let room_id = room.room_id().to_string();
            let content = &value["content"];
            let changed = if content["m.relates_to"]["rel_type"] == "m.replace" {
                let Some(target) = content["m.relates_to"]["event_id"].as_str() else { return };
                let Some(card) = Card::from_content(&content["m.new_content"]) else { return };
                edited(&room_id, target, sender, card)
            } else {
                let Some(card) = Card::from_content(content) else { return };
                let name = match OwnedUserId::try_from(sender) {
                    Ok(user) => room
                        .get_member_no_sync(&user)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|m| m.display_name().map(str::to_owned))
                        .unwrap_or_else(|| user.localpart().to_owned()),
                    Err(_) => return,
                };
                posted(&room_id, event_id, sender, &name, card)
            };
            if changed {
                Cx::post_action(PendingQuestionsChanged);
            }
        },
    );
}

fn reset(owner: String) {
    if let Ok(mut state) = STATE.lock() {
        *state = State { owner, ..Default::default() };
    }
}
pub fn forget_identity() {
    reset(String::new());
}

/// A card message arrived. Returns whether anything changed.
pub fn posted(room_id: &str, card_event: &str, agent: &str, agent_name: &str, card: Card) -> bool {
    let Ok(mut state) = STATE.lock() else { return false };
    if state.cards.iter().any(|c| c.room_id == room_id && c.card_event == card_event) {
        return false;
    }
    // The same card id from another event is never a second card.
    if state.cards.iter().any(|c| c.card.id == card.id) {
        return false;
    }
    let mut entry = QuestionCard {
        room_id: room_id.into(),
        card_event: card_event.into(),
        agent: agent.into(),
        agent_name: agent_name.into(),
        card,
        local: LocalAnswer::None,
        choices: BTreeMap::new(),
        texts: BTreeMap::new(),
    };
    let early: Vec<_> = state
        .early_edits
        .iter()
        .filter(|(room, target, sender, _)| room == room_id && target == card_event && sender == agent)
        .map(|(.., card)| card.clone())
        .collect();
    state.early_edits.retain(|(room, target, ..)| !(room == room_id && target == card_event));
    for card in early {
        apply_edit(&mut entry, card);
    }
    state.cards.push(entry);
    if state.cards.len() > MAX_CARDS {
        state.cards.remove(0);
    }
    true
}

/// The Agent edited its card (answered or expired). Only the card's own
/// sender can change it, and only to a settled state of the same card.
pub fn edited(room_id: &str, card_event: &str, sender: &str, card: Card) -> bool {
    let Ok(mut state) = STATE.lock() else { return false };
    let Some(entry) = state
        .cards
        .iter_mut()
        .find(|c| c.room_id == room_id && c.card_event == card_event)
    else {
        if state.early_edits.len() < 64 {
            state.early_edits.push((room_id.into(), card_event.into(), sender.into(), card));
        }
        return false;
    };
    if entry.agent != sender {
        return false;
    }
    apply_edit(entry, card)
}
fn apply_edit(entry: &mut QuestionCard, card: Card) -> bool {
    let same = card.id == entry.card.id
        && card.trigger_event_id == entry.card.trigger_event_id
        && card.questions == entry.card.questions;
    if !same || card.state == CardState::Pending || entry.card.state != CardState::Pending {
        return false;
    }
    entry.card = card;
    true
}

/// The card raised by the message `event_id` in `room`. In the room timeline a
/// thread's root also carries its thread's cards, since the triggering reply is
/// not shown there. An open card wins over a settled one, then the newest.
pub fn for_event(room: &RoomId, event_id: &str, include_thread_root: bool) -> Option<QuestionCard> {
    let state = STATE.lock().ok()?;
    let now = now_ms();
    let mut matching: Vec<&QuestionCard> = state
        .cards
        .iter()
        .filter(|c| c.room_id == room.as_str())
        .filter(|c| {
            c.card.trigger_event_id == event_id
                || (include_thread_root && c.card.thread_root == event_id && c.card.thread_root != c.card.trigger_event_id)
        })
        .collect();
    matching.sort_by_key(|c| c.card.is_open(now));
    matching.last().map(|c| (*c).clone())
}
pub fn get(card_id: &str) -> Option<QuestionCard> {
    STATE.lock().ok()?.cards.iter().find(|c| c.card.id == card_id).cloned()
}

fn with_card<T>(card_id: &str, f: impl FnOnce(&mut QuestionCard) -> T) -> Option<T> {
    let mut state = STATE.lock().ok()?;
    state.cards.iter_mut().find(|c| c.card.id == card_id).map(f)
}

/// A click on an option. On a card with one question the click is the
/// answer itself; otherwise it marks the choice until Send.
pub fn choose(card_id: &str, question: &str, label: &str) -> Option<QuestionAnswerRequested> {
    let single = with_card(card_id, |c| {
        if !c.open(now_ms()) || !c.card.questions.iter().any(|q| q.id == question && q.options.iter().any(|o| o.label == label)) {
            return None;
        }
        c.texts.remove(question);
        c.choices.insert(question.into(), label.into());
        Some(c.card.questions.len() == 1)
    })??;
    if single { submit(card_id) } else { None }
}
/// Typed text replaces the question's chosen option.
pub fn type_text(card_id: &str, question: &str, text: &str) {
    with_card(card_id, |c| {
        if !c.open(now_ms()) || !c.card.questions.iter().any(|q| q.id == question && q.accepts_text()) {
            return;
        }
        if text.trim().is_empty() {
            c.texts.remove(question);
        } else {
            c.choices.remove(question);
            c.texts.insert(question.into(), text.into());
        }
    });
}
/// The answer to send now, once every question has one. Marks it sending so
/// a second click cannot send it twice.
pub fn submit(card_id: &str) -> Option<QuestionAnswerRequested> {
    with_card(card_id, |c| {
        if !c.open(now_ms()) || !c.ready() {
            return None;
        }
        let content = c.card.answer_content(&c.card_event, &c.answers()).ok()?;
        c.local = LocalAnswer::Sending;
        Some(QuestionAnswerRequested {
            room_id: c.room_id.clone(),
            card_id: c.card.id.clone(),
            agent: c.agent.clone(),
            content,
        })
    })?
}
/// Record the outcome of an answer this client sent.
pub fn sent(update: &QuestionAnswerSent) {
    with_card(&update.card_id, |c| {
        c.local = match &update.result {
            Ok(()) => LocalAnswer::Sent,
            Err(code) => LocalAnswer::Failed(code.clone()),
        };
    });
}

/// Send one answer as a room event, encrypted whenever the room is.
pub fn send(request: QuestionAnswerRequested) {
    crate::sliding_sync::spawn_async_task(async move {
        let result = async {
            let client = crate::sliding_sync::get_client().ok_or("matrix_sign_in_required")?;
            let room_id = OwnedRoomId::try_from(request.room_id.as_str()).map_err(|_| "room_unavailable")?;
            let room = client.get_room(&room_id).ok_or("room_unavailable")?;
            // The room's own encryption state must be known first; an unknown
            // state could otherwise send plaintext into an encrypted room.
            room.latest_encryption_state().await.map_err(|_| "room_state_unavailable")?;
            // Fresh device keys for the Agent, so its device can read the answer.
            if let Ok(agent) = OwnedUserId::try_from(request.agent.as_str()) {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    client.encryption().request_user_identity(&agent),
                )
                .await;
            }
            // One transaction per card and answer: a retry of the same answer
            // returns the original event instead of sending a second one.
            let digest = blake3::hash(format!("{}\n{}", request.card_id, request.content).as_bytes());
            let txn: matrix_sdk::ruma::OwnedTransactionId =
                format!("hagency_answer_{}", &digest.to_hex()[..32]).into();
            room.send_raw(protocol::ANSWER_EVENT, request.content.clone())
                .with_transaction_id(&txn)
                .await
                .map(|_| ())
                .map_err(|_| "answer_send_failed")
        }
        .await
        .map_err(str::to_owned);
        Cx::post_action(QuestionAnswerSent { card_id: request.card_id, result });
    });
}

/// New open cards to announce once each, with the Agent's name.
pub fn take_announcements() -> Vec<(String, String)> {
    let Ok(mut state) = STATE.lock() else { return Vec::new() };
    let now = now_ms();
    let fresh: Vec<(String, String)> = state
        .cards
        .iter()
        .filter(|c| c.open(now) && !state.announced.contains(&c.card.id) && c.agent != state.owner)
        .map(|c| (c.card.id.clone(), c.agent_name.clone()))
        .collect();
    for (id, _) in &fresh {
        state.announced.insert(id.clone());
    }
    fresh
}

/// Everything the card widget draws for one card at `now_ms`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionView {
    pub card_id: String,
    pub title: String,
    pub status: String,
    pub open: bool,
    pub questions: Vec<QuestionRow>,
    pub send_visible: bool,
    pub can_send: bool,
    pub hint: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionRow {
    pub id: String,
    pub header: String,
    pub question: String,
    /// Option labels with whether each is the current choice.
    pub options: Vec<(String, bool)>,
    /// The options' descriptions, one per line.
    pub details: String,
    /// The typed text, when the question takes text and the card is open.
    pub text: Option<String>,
    /// The published answer, once answered.
    pub answer: Option<String>,
}

pub fn view(c: &QuestionCard, now_ms: u64) -> QuestionView {
    let open = c.open(now_ms);
    let expired = c.card.state == CardState::Expired
        || (c.card.state == CardState::Pending && now_ms >= c.card.expires_at_ms);
    use crate::i18n::{format as tf, tr};
    let status = match (&c.card.state, &c.local) {
        (CardState::Answered, _) => tf(
            "Answered by {0}",
            &[("0", c.card.answered_by.clone().unwrap_or_default())],
        ),
        _ if expired => tr("Expired · the Agent continued on its own").to_owned(),
        (_, LocalAnswer::Sending) => tr("Sending…").to_owned(),
        (_, LocalAnswer::Sent) => tr("Sent · waiting for the Agent").to_owned(),
        (_, LocalAnswer::Failed(_)) => tr("Not sent · try again").to_owned(),
        _ => tr("Waiting for an answer").to_owned(),
    };
    let questions = c
        .card
        .questions
        .iter()
        .map(|q| QuestionRow {
            id: q.id.clone(),
            header: q.header.clone(),
            question: q.question.clone(),
            options: q
                .options
                .iter()
                .map(|o| (o.label.clone(), c.choices.get(&q.id) == Some(&o.label)))
                .collect(),
            details: q
                .options
                .iter()
                .filter(|o| !o.description.is_empty())
                .map(|o| format!("{} — {}", o.label, o.description))
                .collect::<Vec<_>>()
                .join("\n"),
            text: (open && q.accepts_text()).then(|| c.texts.get(&q.id).cloned().unwrap_or_default()),
            answer: (c.card.state == CardState::Answered).then(|| match c.card.answers.get(&q.id) {
                Some(AnswerValue::Option(label)) => label.clone(),
                Some(AnswerValue::Text(text)) => text.clone(),
                None => "—".to_owned(),
            }),
        })
        .collect::<Vec<_>>();
    let typed = c.card.questions.iter().any(|q| q.accepts_text());
    let mut hint = if open {
        use chrono::TimeZone;
        // Questions in one run share one wait, so a card states its own end.
        let by = chrono::Local
            .timestamp_millis_opt(c.card.expires_at_ms as i64)
            .single()
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default();
        tf(
            "Any member of this room can answer. With no answer by {0}, the Agent continues on its own.",
            &[("0", by)],
        )
    } else {
        String::new()
    };
    if c.card.withheld > 0 && c.card.state == CardState::Pending {
        if !hint.is_empty() {
            hint.push('\n');
        }
        hint.push_str(tr("A private question was not shown here and gets no answer."));
    }
    QuestionView {
        card_id: c.card.id.clone(),
        title: tf("{0} asks", &[("0", c.agent_name.clone())]),
        status,
        open,
        send_visible: open && (questions.len() > 1 || typed),
        can_send: open && c.ready(),
        questions,
        hint,
    }
}

/// A short notification naming the Agent, never the question text itself.
pub fn announcement(agent_name: &str) -> String {
    crate::i18n::format(
        "{0} asks a question. Open the room to answer it under your message.",
        &[("0", agent_name.to_owned())],
    )
}

#[cfg(test)]
pub(crate) fn test_reset() {
    reset("@me:example.test".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::hagency::agent_questions::{CardOption, CardQuestion};

    fn card(questions: usize) -> Card {
        let question = |id: &str| CardQuestion {
            id: id.into(),
            header: "Database".into(),
            question: format!("Which database for {id}?"),
            options: vec![
                CardOption { label: "Postgres".into(), description: "Durable.".into() },
                CardOption { label: "SQLite".into(), description: String::new() },
            ],
            other: true,
        };
        Card {
            version: 1,
            id: format!("aq_{}", "c".repeat(64)),
            trigger_event_id: "$trigger".into(),
            thread_root: "$root".into(),
            expires_at_ms: now_ms() + 600_000,
            questions: (0..questions).map(|i| question(&format!("q{i}"))).collect(),
            withheld: 0,
            state: CardState::Pending,
            answered_by: None,
            answers: BTreeMap::new(),
        }
    }
    const AGENT: &str = "@_hagency_agt_one:example.test";

    #[test]
    fn single_question_click_is_the_answer_and_sends_once() {
        let _guard = serial();
        test_reset();
        let room = RoomId::parse("!room:example.test").unwrap();
        assert!(posted(room.as_str(), "$card", AGENT, "小黄瓜", card(1)));
        assert!(!posted(room.as_str(), "$card", AGENT, "小黄瓜", card(1)), "the same card arrives once");
        let shown = for_event(&room, "$trigger", false).unwrap();
        let v = view(&shown, now_ms());
        assert!(v.open && v.send_visible, "typed Other text needs the Send button");
        assert_eq!(v.questions[0].options, vec![("Postgres".into(), false), ("SQLite".into(), false)]);
        assert!(for_event(&room, "$root", false).is_none(), "a thread timeline matches the trigger only");
        assert!(for_event(&room, "$root", true).is_some(), "the room timeline shows it on the thread root");
        let request = choose(shown.id(), "q0", "SQLite").expect("one click answers a one-question card");
        assert_eq!(request.content["answers"], serde_json::json!({"q0":{"option":"SQLite"}}));
        assert_eq!(request.content["m.relates_to"]["event_id"], "$card");
        assert!(choose(shown.id(), "q0", "Postgres").is_none(), "nothing more while sending");
        assert!(submit(shown.id()).is_none());
        sent(&QuestionAnswerSent { card_id: shown.id().into(), result: Err("answer_send_failed".into()) });
        assert!(view(&get(shown.id()).unwrap(), now_ms()).status.starts_with("Not sent"));
        assert!(choose(shown.id(), "q0", "Postgres").is_some(), "a failed send can be retried");
        sent(&QuestionAnswerSent { card_id: shown.id().into(), result: Ok(()) });
        assert!(view(&get(shown.id()).unwrap(), now_ms()).status.starts_with("Sent"));
    }

    #[test]
    fn several_questions_wait_for_send_and_typed_text_replaces_a_choice() {
        let _guard = serial();
        test_reset();
        posted("!room:example.test", "$card", AGENT, "Agent", card(2));
        let id = format!("aq_{}", "c".repeat(64));
        assert!(choose(&id, "q0", "Postgres").is_none(), "a choice is not yet the answer");
        assert!(!get(&id).unwrap().ready());
        assert!(submit(&id).is_none());
        type_text(&id, "q1", "  CockroachDB ");
        let c = get(&id).unwrap();
        assert!(c.ready() && view(&c, now_ms()).can_send);
        type_text(&id, "q0", "MySQL");
        assert_eq!(get(&id).unwrap().choices.get("q0"), None, "typed text replaces the choice");
        let request = submit(&id).unwrap();
        assert_eq!(request.content["answers"], serde_json::json!({"q0":{"text":"MySQL"},"q1":{"text":"CockroachDB"}}));
    }

    #[test]
    fn only_the_agents_own_edit_settles_its_card_and_expiry_shows_locally() {
        let _guard = serial();
        test_reset();
        let mut early = card(1);
        early.id = format!("aq_{}", "d".repeat(64));
        let answered = early.answered("@alice:example.test", BTreeMap::from([("q0".into(), AnswerValue::Option("SQLite".into()))]));
        // An edit that arrives before its card is applied once the card does.
        assert!(!edited("!room:example.test", "$early", AGENT, answered.clone()));
        assert!(posted("!room:example.test", "$early", AGENT, "Agent", early.clone()));
        let c = get(&early.id).unwrap();
        assert_eq!(c.card.state, CardState::Answered);
        let v = view(&c, now_ms());
        assert!(!v.open && v.status.starts_with("Answered by @alice:example.test"));
        assert_eq!(v.questions[0].answer.as_deref(), Some("SQLite"));
        // Somebody else cannot settle a card, and a settled card stays settled.
        let other = card(1);
        posted("!room:example.test", "$card", AGENT, "Agent", other.clone());
        assert!(!edited("!room:example.test", "$card", "@_hagency_agt_two:example.test", other.expired()));
        assert!(edited("!room:example.test", "$card", AGENT, other.expired()));
        assert!(!edited("!room:example.test", "$card", AGENT, other.answered("@bob:example.test", BTreeMap::from([("q0".into(), AnswerValue::Option("SQLite".into()))]))));
        assert!(view(&get(&other.id).unwrap(), now_ms()).status.starts_with("Expired"));
        // A card past its time reads as expired before the Agent's edit arrives.
        let mut late = card(1);
        late.id = format!("aq_{}", "e".repeat(64));
        late.trigger_event_id = "$late".into();
        posted("!room:example.test", "$latecard", AGENT, "Agent", late.clone());
        let c = get(&late.id).unwrap();
        let v = view(&c, late.expires_at_ms + 1);
        assert!(!v.open && v.status.starts_with("Expired") && !v.send_visible);
    }

    static SERIAL: Mutex<()> = Mutex::new(());
    pub(super) fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }
}
