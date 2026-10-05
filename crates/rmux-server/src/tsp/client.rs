// Ported from tmux tty.c, tty-keys.c @ 8f25579c
use super::wire::Hello;
use crate::ids::{PaneId, TimerId};
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Capability {
    #[default]
    Unknown,
    Probing,
    Unsupported,
    V1(Hello),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestOwner {
    Detection,
    Replay { pane: PaneId, projection: u64 },
}
#[derive(Clone, Debug)]
pub struct Request {
    pub token: u64,
    pub generation: u64,
    pub owner: RequestOwner,
    pub completed: bool,
    pub hello: Option<Hello>,
}
#[derive(Debug)]
pub struct ClientTspState {
    pub capability: Capability,
    pub generation: u64,
    pub requests: VecDeque<Request>,
    pub timer: Option<TimerId>,
    pub projection: Option<super::projection::Projection>,
    pub projection_generation: u64,
    pub confirmed_blobs: BTreeSet<String>,
    pub visible: bool,
    pub diagnostic: Option<String>,
    pub rebuild_attempted: bool,
    pub failed_logical: Option<String>,
    pub transition_pane: Option<PaneId>,
    next_request: u64,
}
impl Default for ClientTspState {
    fn default() -> Self {
        Self {
            capability: Capability::Unknown,
            generation: 0,
            requests: VecDeque::new(),
            timer: None,
            projection: None,
            projection_generation: 0,
            confirmed_blobs: BTreeSet::new(),
            visible: true,
            diagnostic: None,
            rebuild_attempted: false,
            failed_logical: None,
            transition_pane: None,
            next_request: 0,
        }
    }
}
impl ClientTspState {
    pub fn format(&self) -> &'static str {
        match self.capability {
            Capability::Unknown | Capability::Probing => "unknown",
            Capability::Unsupported => "no",
            Capability::V1(_) => "v1",
        }
    }
    pub fn hello(&self) -> Option<&Hello> {
        if let Capability::V1(hello) = &self.capability {
            Some(hello)
        } else {
            None
        }
    }
    pub fn invalidate(&mut self, generation: u64) {
        self.generation = generation;
        self.capability = Capability::Unknown;
        self.requests.clear();
        self.confirmed_blobs.clear();
        self.projection = None;
        self.visible = true;
        self.diagnostic = None;
        self.rebuild_attempted = false;
        self.failed_logical = None;
        self.transition_pane = None;
    }
    pub fn request(&mut self, owner: RequestOwner) -> u64 {
        self.next_request = self.next_request.wrapping_add(1).max(1);
        let token = self.next_request;
        if owner == RequestOwner::Detection {
            self.capability = Capability::Probing;
        }
        self.requests.push_back(Request {
            token,
            generation: self.generation,
            owner,
            completed: false,
            hello: None,
        });
        token
    }
    pub fn accept_hello(&mut self, hello: Hello) {
        let Some(request) = self.requests.front_mut() else {
            return;
        };
        if request.completed
            || request.generation != self.generation
            || hello.v != 1
            || hello.credits == 0
            || hello.apc < 64
        {
            return;
        }
        request.hello = Some(hello);
    }
    pub fn sentinel(&mut self, token: u64) -> Option<RequestOwner> {
        let request = self.requests.front()?;
        if request.token != token {
            return None;
        }
        let request = self.requests.pop_front()?;
        if request.generation != self.generation {
            return None;
        }
        if request.owner == RequestOwner::Detection && !request.completed {
            self.capability = request
                .hello
                .map_or(Capability::Unsupported, Capability::V1);
        }
        Some(request.owner)
    }
    pub fn expire(&mut self, token: u64) {
        if let Some(request) = self
            .requests
            .iter_mut()
            .find(|r| r.token == token && r.generation == self.generation)
        {
            request.completed = true;
            request.hello = None;
            if request.owner == RequestOwner::Detection {
                self.capability = Capability::Unsupported;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn hello() -> Hello {
        serde_json::from_value(json!({"v":1,"kinds":["text"],"credits":2})).unwrap()
    }
    #[test]
    fn late_hello_cannot_revive_expired_probe() {
        let mut c = ClientTspState::default();
        let token = c.request(RequestOwner::Detection);
        c.expire(token);
        c.accept_hello(hello());
        assert_eq!(c.sentinel(token), Some(RequestOwner::Detection));
        assert_eq!(c.format(), "no");
    }
    #[test]
    fn cache_is_per_generation_and_sentinel_owned() {
        let mut c = ClientTspState::default();
        let token = c.request(RequestOwner::Detection);
        c.accept_hello(hello());
        assert_eq!(c.sentinel(token + 1), None);
        c.sentinel(token);
        assert_eq!(c.format(), "v1");
        c.invalidate(2);
        assert_eq!(c.format(), "unknown");
        assert_eq!(c.sentinel(token), None);
    }
}
