//! Account- and room-scoped Agent presentation identities from the server.
//! These observations never grant message, work, or room authority.
use makepad_widgets::Cx;
use matrix_sdk::ruma::{OwnedRoomId, OwnedUserId, RoomId, UserId};
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

const CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_ROOMS: usize = 32;

#[derive(Clone, PartialEq, Eq)]
struct Pin { owner: OwnedUserId, origin: String, epoch: u64 }
struct Entry {
    pin: Pin,
    request: u64,
    at: Instant,
    agents: HashSet<OwnedUserId>,
}
#[derive(Default)]
struct Cache { next: u64, rooms: HashMap<OwnedRoomId, Entry> }
impl Cache {
    fn begin(&mut self, pin: Pin, room: OwnedRoomId, now: Instant) -> Option<u64> {
        if self.rooms.get(&room).is_some_and(|entry|
            entry.pin == pin && now.saturating_duration_since(entry.at) < CACHE_TTL) {
            return None;
        }
        if !self.rooms.contains_key(&room) && self.rooms.len() >= MAX_ROOMS {
            let oldest = self.rooms.iter().min_by_key(|(_, entry)| entry.at).map(|(room, _)| room.clone())?;
            self.rooms.remove(&oldest);
        }
        self.next = self.next.checked_add(1)?;
        let request = self.next;
        self.rooms.insert(room, Entry { pin, request, at: now, agents: HashSet::new() });
        Some(request)
    }
    fn finish(&mut self, pin: &Pin, room: &RoomId, request: u64,
        agents: HashSet<OwnedUserId>, now: Instant) -> bool {
        let Some(entry) = self.rooms.get_mut(room) else { return false };
        if &entry.pin != pin || entry.request != request { return false; }
        entry.at = now;
        entry.agents = agents;
        true
    }
    fn contains(&self, pin: &Pin, room: &RoomId, agent: &UserId, now: Instant) -> bool {
        self.rooms.get(room).is_some_and(|entry| entry.pin == *pin
            && now.saturating_duration_since(entry.at) < CACHE_TTL && entry.agents.contains(agent))
    }
}
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

#[derive(Debug)]
pub(crate) struct RoomRosterChanged { pub room: OwnedRoomId, pub epoch: u64 }

fn current_pin() -> Option<Pin> {
    let epoch = crate::account_session::epoch();
    if crate::sliding_sync::account_changing() { return None; }
    let client = crate::sliding_sync::get_client()?;
    let owner = client.user_id()?.to_owned();
    let origin = client.homeserver().to_string();
    if epoch != crate::account_session::epoch() || crate::sliding_sync::account_changing() { return None; }
    Some(Pin { owner, origin, epoch })
}

/// Refresh off the UI thread. A result from a prior account or request cannot
/// replace the current room roster. Failures retain ordinary Matrix rendering.
pub(crate) fn request(room: &OwnedRoomId) {
    let Some(pin) = current_pin() else { return };
    let room = room.clone();
    let Some(request) = CACHE.lock().unwrap_or_else(|e| e.into_inner())
        .begin(pin.clone(), room.clone(), Instant::now()) else { return };
    crate::sliding_sync::spawn_async_task(async move {
        let result = tokio::time::timeout(Duration::from_secs(30),
            crate::home::mobile_chat_info::fetch_member_owners(room.as_str(), &pin.owner)).await;
        let agents = match result {
            Ok(Ok((owners, _))) => owners.into_keys().collect(),
            _ => HashSet::new(),
        };
        if current_pin().as_ref() != Some(&pin) { return; }
        let changed = CACHE.lock().unwrap_or_else(|e| e.into_inner())
            .finish(&pin, &room, request, agents, Instant::now());
        if changed { Cx::post_action(RoomRosterChanged { room, epoch: pin.epoch }); }
    });
}

pub(crate) fn contains(room: &RoomId, user: &UserId) -> bool {
    let Some(pin) = current_pin() else { return false };
    let now = Instant::now();
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let known = cache.contains(&pin, room, user, now);
    let stale = cache.rooms.get(room).is_none_or(|entry| entry.pin != pin
        || now.saturating_duration_since(entry.at) >= CACHE_TTL);
    drop(cache);
    if stale { request(&room.to_owned()); }
    known
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roster_results_are_scoped_and_stale_requests_are_rejected() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let pin = Pin { owner: "@owner:test".try_into().unwrap(), origin: "https://test/".into(), epoch: 1 };
        let room: OwnedRoomId = "!room:test".try_into().unwrap();
        let other: OwnedRoomId = "!other:test".try_into().unwrap();
        let agent: OwnedUserId = "@dragon:test".try_into().unwrap();
        let first = cache.begin(pin.clone(), room.clone(), now).unwrap();
        assert!(cache.begin(pin.clone(), room.clone(), now).is_none());
        assert!(cache.finish(&pin, &room, first, [agent.clone()].into(), now));
        assert!(cache.contains(&pin, &room, &agent, now));
        assert!(!cache.contains(&pin, &other, &agent, now));
        let mut next_pin = pin.clone(); next_pin.epoch += 1;
        let next = cache.begin(next_pin.clone(), room.clone(), now).unwrap();
        assert!(!cache.finish(&pin, &room, first, [agent.clone()].into(), now));
        assert!(!cache.contains(&next_pin, &room, &agent, now));
        assert!(cache.finish(&next_pin, &room, next, [agent.clone()].into(), now));
        assert!(!cache.contains(&pin, &room, &agent, now));
        assert!(cache.contains(&next_pin, &room, &agent, now));
        assert!(!cache.contains(&next_pin, &room, &agent, now + CACHE_TTL));
    }
}
