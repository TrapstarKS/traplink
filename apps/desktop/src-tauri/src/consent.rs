use serde::Serialize;
use std::{
    collections::HashMap,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const PROTOCOL: u32 = 3;
pub const PROMPT_LIFETIME: Duration = Duration::from_secs(60);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(65);
pub const COOLDOWN: Duration = Duration::from_secs(30);

pub const DECLINED: &str = "Input permission was declined on the other computer.";
pub const BUSY: &str = "A permission request for this computer is already waiting.";
pub const OUTDATED: &str =
    "The other computer runs an older TrapLink that cannot ask for input permission.";
pub const EXPIRED: &str = "Nobody answered the input permission request on the other computer.";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentRequest {
    pub peer_id: String,
    pub peer_name: String,
    pub request: String,
    pub flow: bool,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Granted,
    Prompt(ConsentRequest),
    Refuse(&'static str),
}

#[derive(Default)]
pub struct ConsentState {
    incoming: HashMap<String, (ConsentRequest, Instant)>,
    declined: HashMap<String, Instant>,
    outgoing: HashMap<String, Instant>,
}

impl ConsentState {
    pub fn receive(
        &mut self,
        request: ConsentRequest,
        already_permitted: bool,
        now: Instant,
    ) -> Decision {
        self.expire(now);
        if already_permitted {
            return Decision::Granted;
        }
        if self.declined.contains_key(&request.peer_id) {
            return Decision::Refuse(DECLINED);
        }
        if self.incoming.contains_key(&request.peer_id) {
            return Decision::Refuse(BUSY);
        }
        self.incoming
            .insert(request.peer_id.clone(), (request.clone(), now));
        Decision::Prompt(request)
    }
    pub fn answer(
        &mut self,
        peer_id: &str,
        request: &str,
        allow: bool,
        now: Instant,
    ) -> Option<ConsentRequest> {
        self.expire(now);
        if !matches!(self.incoming.get(peer_id), Some((open, _)) if open.request == request) {
            return None;
        }
        let (open, _) = self.incoming.remove(peer_id)?;
        if !allow {
            self.declined.insert(peer_id.into(), now);
        }
        Some(open)
    }
    pub fn pending(&mut self, now: Instant) -> Vec<ConsentRequest> {
        self.expire(now);
        let mut open: Vec<_> = self.incoming.values().cloned().collect();
        open.sort_by_key(|(_, at)| *at);
        open.into_iter().map(|(request, _)| request).collect()
    }
    pub fn forget(&mut self, peer_id: &str) {
        self.incoming.remove(peer_id);
    }
    pub fn begin_outgoing(&mut self, peer_id: &str, now: Instant) -> Result<(), &'static str> {
        self.expire(now);
        if self.outgoing.contains_key(peer_id) {
            return Err(BUSY);
        }
        self.outgoing.insert(peer_id.into(), now);
        Ok(())
    }
    pub fn end_outgoing(&mut self, peer_id: &str) {
        self.outgoing.remove(peer_id);
    }
    fn expire(&mut self, now: Instant) {
        let older = |at: &Instant, limit: Duration| now.saturating_duration_since(*at) >= limit;
        self.incoming
            .retain(|_, (_, at)| !older(at, PROMPT_LIFETIME));
        self.declined.retain(|_, at| !older(at, COOLDOWN));
        self.outgoing
            .retain(|_, at| !older(at, REQUEST_TIMEOUT + Duration::from_secs(5)));
    }
}

type Notifier = Box<dyn Fn(&ConsentRequest) + Send + Sync>;
static NOTIFIER: OnceLock<Notifier> = OnceLock::new();

pub fn set_notifier(notifier: Notifier) {
    let _ = NOTIFIER.set(notifier);
}
pub fn notify(request: &ConsentRequest) {
    if let Some(notifier) = NOTIFIER.get() {
        notifier(request);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(peer: &str, request: &str) -> ConsentRequest {
        ConsentRequest {
            peer_id: peer.into(),
            peer_name: format!("{peer} name"),
            request: request.into(),
            flow: true,
        }
    }

    #[test]
    fn already_permitted_peers_are_acknowledged_without_a_prompt() {
        let mut state = ConsentState::default();
        let now = Instant::now();
        assert_eq!(state.receive(ask("a", "1"), true, now), Decision::Granted);
        assert!(state.pending(now).is_empty());
    }

    #[test]
    fn one_prompt_per_peer_and_answers_must_match_the_open_request() {
        let mut state = ConsentState::default();
        let now = Instant::now();
        assert_eq!(
            state.receive(ask("a", "1"), false, now),
            Decision::Prompt(ask("a", "1"))
        );
        assert_eq!(
            state.receive(ask("a", "2"), false, now),
            Decision::Refuse(BUSY)
        );
        assert!(matches!(
            state.receive(ask("b", "3"), false, now),
            Decision::Prompt(_)
        ));
        assert_eq!(state.pending(now).len(), 2);
        assert_eq!(state.answer("a", "2", true, now), None, "stale request id");
        assert_eq!(state.answer("a", "1", true, now), Some(ask("a", "1")));
        assert_eq!(state.answer("a", "1", true, now), None, "answered once");
        assert_eq!(state.pending(now), vec![ask("b", "3")]);
    }

    #[test]
    fn declining_starts_a_cooldown_and_prompts_expire() {
        let mut state = ConsentState::default();
        let t0 = Instant::now();
        state.receive(ask("a", "1"), false, t0);
        assert!(state.answer("a", "1", false, t0).is_some());
        assert_eq!(
            state.receive(ask("a", "2"), false, t0 + Duration::from_secs(5)),
            Decision::Refuse(DECLINED)
        );
        assert_eq!(
            state.receive(ask("a", "3"), true, t0 + Duration::from_secs(6)),
            Decision::Granted
        );
        let after = t0 + COOLDOWN;
        assert!(matches!(
            state.receive(ask("a", "4"), false, after),
            Decision::Prompt(_)
        ));
        assert!(state.pending(after + PROMPT_LIFETIME).is_empty());
        assert_eq!(state.answer("a", "4", true, after + PROMPT_LIFETIME), None);
    }

    #[test]
    fn requester_keeps_one_request_in_flight_per_peer() {
        let mut state = ConsentState::default();
        let now = Instant::now();
        assert!(state.begin_outgoing("a", now).is_ok());
        assert_eq!(state.begin_outgoing("a", now), Err(BUSY));
        assert!(state.begin_outgoing("b", now).is_ok());
        state.end_outgoing("a");
        assert!(state.begin_outgoing("a", now).is_ok());
        assert!(state
            .begin_outgoing("b", now + REQUEST_TIMEOUT + Duration::from_secs(5))
            .is_ok());
    }

    #[test]
    fn forgetting_a_peer_closes_its_prompt() {
        let mut state = ConsentState::default();
        let now = Instant::now();
        state.receive(ask("a", "1"), false, now);
        state.forget("a");
        assert!(state.pending(now).is_empty());
    }
}
