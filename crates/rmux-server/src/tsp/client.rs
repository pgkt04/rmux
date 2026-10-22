// Ported from tmux tty.c, tty-keys.c @ 8f25579c
use super::wire::Hello;
use crate::ids::{PaneId, TimerId};
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

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
    /// The blob ids a `q blobs` in this request asked about.
    pub blobs: Vec<String>,
}
/// Where a client keeps one of its projections; a hidden one by its
/// projection generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Shown,
    Covered,
    Hidden(u64),
}
/// The most hidden projections a client keeps, whatever its host offers.
pub const RMUX_HIDDEN_MAX: usize = 8;
/// How long a detection probe waits for the client's DA1 reply.
pub const PROBE_DEADLINE: Duration = Duration::from_secs(1);
/// The waits before a probe that passed its deadline unanswered goes out
/// again. Answers match probes in order, so a retry recovers a client that is
/// late, not a lost answer: a late host still answers every probe it saw.
pub const PROBE_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(10),
];
/// The detection probes sent after one passed its deadline unanswered. A
/// client that answers the DA1 without a hello is not TSP; it gets none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeRetry {
    /// Timed retries sent so far, one per `PROBE_RETRY_DELAYS` entry.
    pub timed: usize,
    /// The one retry on the client's next input or resize was sent.
    pub on_activity: bool,
    /// The newest probe passed its deadline without a DA1 reply: the client
    /// is `Unsupported` until a late or a retried answer says otherwise.
    pub unanswered: bool,
}
#[derive(Debug)]
pub struct ClientTspState {
    pub capability: Capability,
    pub generation: u64,
    pub requests: VecDeque<Request>,
    pub timer: Option<TimerId>,
    pub probe: ProbeRetry,
    pub projection: Option<super::projection::Projection>,
    /// The inline projection under a screen `projection`. The host keeps an
    /// inline surface live under a screen surface, so closing the screen shows
    /// it again without a replay.
    pub covered: Option<super::projection::Projection>,
    /// The inline projections of windows the client left: suspended on the
    /// host, which shows the grid, so coming back resumes one without a
    /// replay. LRU order, front = least recently hidden; at most
    /// `hidden_cap()` entries, one per pane.
    pub hidden: Vec<super::projection::Projection>,
    pub projection_generation: u64,
    pub confirmed_blobs: BTreeSet<String>,
    pub(crate) projection_hello: Option<serde_json::Value>,
    pub visible: bool,
    pub diagnostic: Option<String>,
    pub rebuild_attempted: bool,
    pub failed_logical: Option<String>,
    /// The TSP host lost the outer surface and the projection was opened
    /// again; no frame of it is acked yet.
    pub host_reopen_unacked: bool,
    pub transition_pane: Option<PaneId>,
    /// Tern rejected the native status bar; this client goes without it.
    pub bar_failed: bool,
    /// The bar segments a click resolves, as last drawn.
    pub bar_segs: Vec<super::status_bar::BarSeg>,
    next_request: u64,
}
impl Default for ClientTspState {
    fn default() -> Self {
        Self {
            capability: Capability::Unknown,
            generation: 0,
            requests: VecDeque::new(),
            timer: None,
            probe: ProbeRetry::default(),
            projection: None,
            covered: None,
            hidden: Vec::new(),
            projection_generation: 0,
            confirmed_blobs: BTreeSet::new(),
            projection_hello: None,
            visible: true,
            diagnostic: None,
            rebuild_attempted: false,
            failed_logical: None,
            host_reopen_unacked: false,
            transition_pane: None,
            bar_failed: false,
            bar_segs: Vec::new(),
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
    /// The host gives the whole pane back to the grid on `["suspend"]`, so a
    /// window switch can keep the inline projection hidden instead of closing
    /// it. rsttyd hides the surface's DOM; Tern 0.8.4 keeps a live suspended
    /// surface's dock strip over the last grid row, and only `x` returns the
    /// full grid, so every other host closes.
    pub fn host_yields_pane_on_suspend(&self) -> bool {
        self.hello()
            .is_some_and(|h| h.term.as_deref() == Some("rsttyd"))
    }
    /// How many projections the client keeps hidden: as many as the host says
    /// it parks (`suspend`, at most `RMUX_HIDDEN_MAX`); one for a host that
    /// yields the pane but keeps a single parked surface; none otherwise.
    pub fn hidden_cap(&self) -> usize {
        match self.hello().and_then(|h| h.suspend) {
            Some(n) if n > 1 => (n as usize).min(RMUX_HIDDEN_MAX),
            _ if self.host_yields_pane_on_suspend() => 1,
            _ => 0,
        }
    }
    /// More than one hidden slot: the host parks every suspended surface, so
    /// only a stale one of the same pane needs closing before another opens.
    pub fn hides_many(&self) -> bool {
        self.hidden_cap() > 1
    }
    pub fn invalidate(&mut self, generation: u64) {
        self.generation = generation;
        self.capability = Capability::Unknown;
        self.requests.clear();
        self.probe = ProbeRetry::default();
        self.confirmed_blobs.clear();
        self.projection_hello = None;
        self.projection = None;
        self.covered = None;
        self.hidden.clear();
        self.visible = true;
        self.diagnostic = None;
        self.rebuild_attempted = false;
        self.failed_logical = None;
        self.host_reopen_unacked = false;
        self.transition_pane = None;
        self.bar_failed = false;
        self.bar_segs.clear();
    }
    pub fn slot(&self, slot: Slot) -> Option<&super::projection::Projection> {
        match slot {
            Slot::Shown => self.projection.as_ref(),
            Slot::Covered => self.covered.as_ref(),
            Slot::Hidden(generation) => self.hidden.iter().find(|p| p.generation == generation),
        }
    }
    pub fn slot_mut(&mut self, slot: Slot) -> Option<&mut super::projection::Projection> {
        match slot {
            Slot::Shown => self.projection.as_mut(),
            Slot::Covered => self.covered.as_mut(),
            Slot::Hidden(generation) => self.hidden.iter_mut().find(|p| p.generation == generation),
        }
    }
    /// The slot whose projection the host knows as `outer`.
    pub fn slot_of(&self, outer: &str) -> Option<Slot> {
        if self.projection.as_ref().is_some_and(|p| p.outer == outer) {
            return Some(Slot::Shown);
        }
        if self.covered.as_ref().is_some_and(|p| p.outer == outer) {
            return Some(Slot::Covered);
        }
        self.hidden
            .iter()
            .find(|p| p.outer == outer)
            .map(|p| Slot::Hidden(p.generation))
    }
    /// Every projection the host has open: shown, covered, hidden.
    pub fn projections(&self) -> impl Iterator<Item = &super::projection::Projection> {
        self.projection
            .iter()
            .chain(&self.covered)
            .chain(&self.hidden)
    }
    pub fn projections_mut(&mut self) -> impl Iterator<Item = &mut super::projection::Projection> {
        self.projection
            .iter_mut()
            .chain(self.covered.iter_mut())
            .chain(self.hidden.iter_mut())
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
            blobs: Vec::new(),
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
            self.probe.unanswered = false;
        }
        Some(request.owner)
    }
    /// Request `token` passed its deadline. A detection probe stays open: a
    /// late answer still counts while it is the newest probe, and the client
    /// falls back to the grid meanwhile. A blob query is closed.
    pub fn expire(&mut self, token: u64) {
        if let Some(request) = self
            .requests
            .iter_mut()
            .find(|r| r.token == token && r.generation == self.generation && !r.completed)
        {
            if request.owner == RequestOwner::Detection {
                self.capability = Capability::Unsupported;
                self.probe.unanswered = true;
            } else {
                request.completed = true;
                request.hello = None;
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
    fn late_hello_revives_an_expired_probe_that_is_still_the_newest() {
        let mut c = ClientTspState::default();
        let token = c.request(RequestOwner::Detection);
        c.expire(token);
        assert_eq!(c.format(), "no");
        assert!(c.probe.unanswered);
        c.accept_hello(hello());
        assert_eq!(c.sentinel(token), Some(RequestOwner::Detection));
        assert_eq!(c.format(), "v1");
        assert!(!c.probe.unanswered);
    }
    #[test]
    fn da1_without_hello_ends_an_expired_probe_as_unsupported() {
        let mut c = ClientTspState::default();
        let token = c.request(RequestOwner::Detection);
        c.expire(token);
        assert_eq!(c.sentinel(token), Some(RequestOwner::Detection));
        assert_eq!(c.format(), "no");
        assert!(!c.probe.unanswered);
    }
    #[test]
    fn cache_is_per_generation_and_sentinel_owned() {
        let mut c = ClientTspState::default();
        let token = c.request(RequestOwner::Detection);
        c.accept_hello(hello());
        assert_eq!(c.sentinel(token + 1), None);
        c.sentinel(token);
        assert_eq!(c.format(), "v1");
        c.projection_hello = Some(json!({"q":"hello","app":"omp"}));
        c.invalidate(2);
        assert_eq!(c.format(), "unknown");
        assert!(c.projection_hello.is_none());
        assert_eq!(c.sentinel(token), None);
    }
}
