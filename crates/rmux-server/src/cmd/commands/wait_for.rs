// Ported from tmux cmd-wait-for.c @ 8f25579c
use super::support::{client_name, fail, item_client};
use crate::cmd::Command;
use crate::cmd::hooks;
use crate::cmd::queue::{self, CmdReturn};
use crate::format::{self, FormatFlags, FormatTree};
use crate::ids::{ClientId, EventSinkId, QueueItemId};
use crate::server::Server;
use crate::server::events::{self, EventPayload};
use rmux_util::bytes::ByteString;
use std::collections::{BTreeMap, VecDeque};

/// `struct wait_channel` (`cmd-wait-for.c:60-71`); `waiters`/`lockers` are FIFO.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WaitChannel {
    pub locked: bool,
    pub woken: bool,
    pub waiters: VecDeque<QueueItemId>,
    pub lockers: VecDeque<QueueItemId>,
}

/// `struct wait_event_item` (`cmd-wait-for.c:46-55`).
#[derive(Debug)]
pub struct WaitEventItem {
    pub item: QueueItemId,
    pub sink: EventSinkId,
    pub name: ByteString,
    pub filter: Option<ByteString>,
    pub verbose: bool,
}

/// `RB_HEAD(wait_channels)`; `BTreeMap` byte order matches `strcmp`.
#[derive(Debug, Default)]
pub struct WaitChannels {
    pub channels: BTreeMap<ByteString, WaitChannel>,
}

/// `channel <name> not locked` (`cmd-wait-for.c:466-468`).
#[derive(Debug, PartialEq, Eq)]
pub struct NotLocked;

/// Outcome of one channel operation; the caller continues the listed items in order.
#[derive(Debug, PartialEq, Eq)]
pub struct WaitOutcome {
    pub retval: CmdReturn,
    pub resumed: Vec<QueueItemId>,
}
impl WaitOutcome {
    fn normal() -> Self {
        Self {
            retval: CmdReturn::Normal,
            resumed: Vec::new(),
        }
    }
    fn wait() -> Self {
        Self {
            retval: CmdReturn::Wait,
            resumed: Vec::new(),
        }
    }
}

impl WaitChannels {
    fn add(&mut self, name: &[u8]) -> &mut WaitChannel {
        self.channels.entry(ByteString::from(name)).or_default()
    }
    /// `cmd_wait_for_remove` (`cmd-wait-for.c:122-136`).
    fn remove(&mut self, name: &[u8]) {
        if let Some(wc) = self.channels.get(name)
            && !wc.locked
            && wc.waiters.is_empty()
            && wc.woken
        {
            rmux_util::log_debug!("remove wait channel {}", String::from_utf8_lossy(name));
            self.channels.remove(name);
        }
    }
    /// `cmd_wait_for_remove_empty` (`cmd-wait-for.c:138-152`).
    fn remove_empty(&mut self, name: &[u8]) {
        if let Some(wc) = self.channels.get(name)
            && !wc.locked
            && !wc.woken
            && wc.waiters.is_empty()
            && wc.lockers.is_empty()
        {
            rmux_util::log_debug!(
                "remove empty wait channel {}",
                String::from_utf8_lossy(name)
            );
            self.channels.remove(name);
        }
    }
    /// `cmd_wait_for_signal` (`cmd-wait-for.c:379-404`).
    pub fn signal(&mut self, name: &[u8]) -> WaitOutcome {
        let wc = self.add(name);
        if wc.waiters.is_empty() && !wc.woken {
            wc.woken = true;
            return WaitOutcome::normal();
        }
        let resumed: Vec<_> = wc.waiters.drain(..).collect();
        self.remove(name);
        WaitOutcome {
            retval: CmdReturn::Normal,
            resumed,
        }
    }
    /// `cmd_wait_for_wait` after the client check (`cmd-wait-for.c:416-432`).
    pub fn wait(&mut self, name: &[u8], item: QueueItemId) -> WaitOutcome {
        let wc = self.add(name);
        if wc.woken {
            self.remove(name);
            return WaitOutcome::normal();
        }
        wc.waiters.push_back(item);
        WaitOutcome::wait()
    }
    /// `cmd_wait_for_lock` after the client check (`cmd-wait-for.c:445-457`).
    pub fn lock(&mut self, name: &[u8], item: QueueItemId) -> WaitOutcome {
        let wc = self.add(name);
        if wc.locked {
            wc.lockers.push_back(item);
            return WaitOutcome::wait();
        }
        wc.locked = true;
        WaitOutcome::normal()
    }
    /// `cmd_wait_for_unlock` (`cmd-wait-for.c:460-481`).
    pub fn unlock(&mut self, name: &[u8]) -> Result<WaitOutcome, NotLocked> {
        let Some(wc) = self.channels.get_mut(name) else {
            return Err(NotLocked);
        };
        if !wc.locked {
            return Err(NotLocked);
        }
        if let Some(first) = wc.lockers.pop_front() {
            return Ok(WaitOutcome {
                retval: CmdReturn::Normal,
                resumed: vec![first],
            });
        }
        wc.locked = false;
        self.remove(name);
        Ok(WaitOutcome::normal())
    }
    /// `cmd_wait_for_wake` (`cmd-wait-for.c:347-377`): first matching waiter, then locker.
    pub fn wake(
        &mut self,
        name: &[u8],
        mut matches: impl FnMut(QueueItemId) -> bool,
    ) -> WaitOutcome {
        let Some(wc) = self.channels.get_mut(name) else {
            return WaitOutcome::normal();
        };
        let found = wc
            .waiters
            .iter()
            .position(|&i| matches(i))
            .map(|p| wc.waiters.remove(p).expect("waiter"))
            .or_else(|| {
                wc.lockers
                    .iter()
                    .position(|&i| matches(i))
                    .map(|p| wc.lockers.remove(p).expect("locker"))
            });
        match found {
            Some(item) => {
                self.remove_empty(name);
                WaitOutcome {
                    retval: CmdReturn::Normal,
                    resumed: vec![item],
                }
            }
            None => WaitOutcome::normal(),
        }
    }
    /// `cmd_wait_for_list` (`cmd-wait-for.c:331-345`): waiters then lockers.
    pub fn list(&self, name: &[u8]) -> impl Iterator<Item = QueueItemId> + '_ {
        self.channels
            .get(name)
            .into_iter()
            .flat_map(|wc| wc.waiters.iter().chain(wc.lockers.iter()).copied())
    }
    /// Channel part of `cmd_wait_for_client_lost` (`cmd-wait-for.c:497-514`).
    pub fn client_lost(&mut self, mut owned: impl FnMut(QueueItemId) -> bool) -> Vec<QueueItemId> {
        let mut resumed = Vec::new();
        let names: Vec<ByteString> = self.channels.keys().cloned().collect();
        for name in names {
            let wc = self.channels.get_mut(&name).expect("channel");
            for list in [&mut wc.waiters, &mut wc.lockers] {
                list.retain(|&i| {
                    let lost = owned(i);
                    if lost {
                        resumed.push(i);
                    }
                    !lost
                });
            }
            self.remove_empty(&name);
        }
        resumed
    }
    /// Channel part of `cmd_wait_for_flush` (`cmd-wait-for.c:527-544`).
    pub fn flush(&mut self) -> Vec<QueueItemId> {
        let mut resumed = Vec::new();
        let names: Vec<ByteString> = self.channels.keys().cloned().collect();
        for name in names {
            let wc = self.channels.get_mut(&name).expect("channel");
            resumed.extend(wc.waiters.drain(..));
            wc.woken = true;
            resumed.extend(wc.lockers.drain(..));
            wc.locked = false;
            self.remove(&name);
        }
        resumed
    }
}

fn resume(server: &mut Server, outcome: WaitOutcome) -> CmdReturn {
    for id in outcome.resumed {
        queue::continue_item(&mut server.queue, id);
    }
    outcome.retval
}

pub fn execute(server: &mut Server, command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    let name = args.string(0).unwrap_or_default();
    if args.has(b'E') != 0 {
        return event(server, name, command, item);
    }
    if args.has(b'l') != 0 {
        let items: Vec<QueueItemId> = server.wait_channels.list(name).collect();
        for wi in items {
            let client = item_client(server, wi);
            let text = client_name(server, client).to_vec();
            queue::print(server, item, &text);
        }
        return CmdReturn::Normal;
    }
    if args.has(b'w') != 0 {
        let wanted = args.get(b'w').unwrap_or_default().to_vec();
        let names: BTreeMap<QueueItemId, Vec<u8>> = server
            .wait_channels
            .list(name)
            .map(|wi| {
                let client = item_client(server, wi);
                (wi, client_name(server, client).to_vec())
            })
            .collect();
        let outcome = server
            .wait_channels
            .wake(name, |wi| names.get(&wi).is_some_and(|n| *n == wanted));
        return resume(server, outcome);
    }
    if args.has(b'S') != 0 {
        let outcome = server.wait_channels.signal(name);
        return resume(server, outcome);
    }
    if args.has(b'L') != 0 {
        if item_client(server, item).is_none() {
            return fail(server, item, b"not able to lock");
        }
        let outcome = server.wait_channels.lock(name, item);
        return resume(server, outcome);
    }
    if args.has(b'U') != 0 {
        return match server.wait_channels.unlock(name) {
            Ok(outcome) => resume(server, outcome),
            Err(NotLocked) => {
                let mut msg = b"channel ".to_vec();
                msg.extend_from_slice(name);
                msg.extend_from_slice(b" not locked");
                fail(server, item, msg)
            }
        };
    }
    if item_client(server, item).is_none() {
        return fail(server, item, b"not able to wait");
    }
    let outcome = server.wait_channels.wait(name, item);
    resume(server, outcome)
}

fn event_client_name(server: &Server, wei: &WaitEventItem) -> Vec<u8> {
    client_name(server, item_client(server, wei.item)).to_vec()
}

/// `cmd_wait_for_event_free` without the list removal (`cmd-wait-for.c:255-262`).
fn event_free(server: &mut Server, wei: WaitEventItem) {
    events::remove_sink(server, wei.sink);
}

/// `cmd_wait_for_event_cb` (`cmd-wait-for.c:225-253`).
fn event_cb(server: &mut Server, payload: &mut EventPayload, sink: EventSinkId) {
    let Some(index) = server.wait_event_items.iter().position(|w| w.sink == sink) else {
        return;
    };
    let (item, verbose, filter) = {
        let wei = &server.wait_event_items[index];
        (wei.item, wei.verbose, wei.filter.clone())
    };
    if verbose {
        let lines: Vec<Vec<u8>> = payload
            .iter()
            .filter(|epi| !epi.name().starts_with(b"_"))
            .map(|epi| {
                let mut line = epi.name().to_vec();
                line.push(b'=');
                line.extend_from_slice(&epi.print(server));
                line
            })
            .collect();
        for line in lines {
            queue::print(server, item, &line);
        }
    }
    if let Some(filter) = filter {
        let client = item_client(server, item);
        let mut ft = FormatTree::create(client, Some(item), 0, FormatFlags::NOJOBS, server);
        payload.add_formats(server, &mut ft, b"");
        let expanded = ft.expand(server, &filter);
        ft.release(server);
        if !format::true_value(Some(&expanded)) {
            return;
        }
    }
    let Some(index) = server.wait_event_items.iter().position(|w| w.sink == sink) else {
        return;
    };
    let wei = server.wait_event_items.remove(index);
    queue::continue_item(&mut server.queue, wei.item);
    event_free(server, wei);
}

/// `cmd_wait_for_event` (`cmd-wait-for.c:264-293`).
fn event(server: &mut Server, name: &[u8], command: &Command, item: QueueItemId) -> CmdReturn {
    let args = &command.args;
    if !hooks::valid_event_name(name) {
        let mut msg = b"invalid event: ".to_vec();
        msg.extend_from_slice(name);
        return fail(server, item, msg);
    }
    if args.has(b'l') != 0 {
        let names: Vec<Vec<u8>> = server
            .wait_event_items
            .iter()
            .filter(|w| w.name.as_bytes() == name)
            .map(|w| event_client_name(server, w))
            .collect();
        for n in names {
            queue::print(server, item, &n);
        }
        return CmdReturn::Normal;
    }
    if args.has(b'w') != 0 {
        let wanted = args.get(b'w').unwrap_or_default();
        let index = server
            .wait_event_items
            .iter()
            .position(|w| w.name.as_bytes() == name && event_client_name(server, w) == wanted);
        return match index {
            Some(index) => {
                let wei = server.wait_event_items.remove(index);
                queue::continue_item(&mut server.queue, wei.item);
                event_free(server, wei);
                CmdReturn::Normal
            }
            None => {
                let mut msg = b"waiter ".to_vec();
                msg.extend_from_slice(wanted);
                msg.extend_from_slice(b" not found");
                fail(server, item, msg)
            }
        };
    }
    if item_client(server, item).is_none() {
        return fail(server, item, b"not able to wait");
    }
    let sink = events::add_sink_with_id(server, name, event_cb);
    server.wait_event_items.push(WaitEventItem {
        item,
        sink,
        name: ByteString::from(name),
        filter: args.get(b'F').map(ByteString::from),
        verbose: args.has(b'v') != 0,
    });
    CmdReturn::Wait
}

/// `cmd_wait_for_client_lost` (`cmd-wait-for.c:483-515`).
pub fn client_lost(server: &mut Server, client: ClientId) {
    let mut index = 0;
    while index < server.wait_event_items.len() {
        if item_client(server, server.wait_event_items[index].item) == Some(client) {
            let wei = server.wait_event_items.remove(index);
            queue::continue_item(&mut server.queue, wei.item);
            event_free(server, wei);
        } else {
            index += 1;
        }
    }
    let owned: Vec<QueueItemId> = server
        .wait_channels
        .channels
        .values()
        .flat_map(|wc| wc.waiters.iter().chain(wc.lockers.iter()).copied())
        .filter(|&wi| item_client(server, wi) == Some(client))
        .collect();
    let resumed = server.wait_channels.client_lost(|wi| owned.contains(&wi));
    for id in resumed {
        queue::continue_item(&mut server.queue, id);
    }
}

/// `cmd_wait_for_flush` (`cmd-wait-for.c:517-545`).
pub fn flush(server: &mut Server) {
    for wei in std::mem::take(&mut server.wait_event_items) {
        queue::continue_item(&mut server.queue, wei.item);
        event_free(server, wei);
    }
    let resumed = server.wait_channels.flush();
    for id in resumed {
        queue::continue_item(&mut server.queue, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{Arena, ArenaId};

    fn ids(n: u32) -> Vec<QueueItemId> {
        let mut arena: Arena<(), QueueItemId> = Arena::new();
        (0..n).map(|_| arena.insert(()).unwrap()).collect()
    }
    fn resumed(ids: &[QueueItemId]) -> WaitOutcome {
        WaitOutcome {
            retval: CmdReturn::Normal,
            resumed: ids.to_vec(),
        }
    }

    #[test]
    fn signal_without_waiters_sets_woken_and_wait_consumes_it() {
        let i = ids(2);
        let mut ch = WaitChannels::default();
        assert_eq!(ch.signal(b"a"), WaitOutcome::normal());
        assert!(ch.channels[b"a".as_slice()].woken);
        assert_eq!(ch.wait(b"a", i[0]), WaitOutcome::normal());
        assert!(ch.channels.is_empty(), "woken channel removed by wait");
        // A second signal on an already woken channel with no waiters removes it.
        ch.signal(b"b");
        assert_eq!(ch.signal(b"b"), WaitOutcome::normal());
        assert!(ch.channels.is_empty());
    }

    #[test]
    fn signal_with_waiters_resumes_fifo_and_keeps_unwoken_channel() {
        let i = ids(3);
        let mut ch = WaitChannels::default();
        assert_eq!(ch.wait(b"a", i[0]), WaitOutcome::wait());
        assert_eq!(ch.wait(b"a", i[1]), WaitOutcome::wait());
        assert_eq!(ch.signal(b"a"), resumed(&[i[0], i[1]]));
        // Not a counting semaphore: the channel stays, unwoken (remove needs woken).
        let wc = &ch.channels[b"a".as_slice()];
        assert!(!wc.woken && wc.waiters.is_empty());
        assert_eq!(ch.wait(b"a", i[2]), WaitOutcome::wait());
    }

    #[test]
    fn lock_unlock_hands_over_without_releasing() {
        let i = ids(3);
        let mut ch = WaitChannels::default();
        assert_eq!(ch.lock(b"l", i[0]), WaitOutcome::normal());
        assert_eq!(ch.lock(b"l", i[1]), WaitOutcome::wait());
        assert_eq!(ch.lock(b"l", i[2]), WaitOutcome::wait());
        assert_eq!(ch.unlock(b"l"), Ok(resumed(&[i[1]])));
        assert!(ch.channels[b"l".as_slice()].locked);
        assert_eq!(ch.unlock(b"l"), Ok(resumed(&[i[2]])));
        assert_eq!(ch.unlock(b"l"), Ok(WaitOutcome::normal()));
        // Unlocked, not woken, no waiters: remove() keeps it (needs woken).
        assert!(ch.channels.contains_key(b"l".as_slice()));
        assert_eq!(ch.unlock(b"l"), Err(NotLocked));
        assert_eq!(ch.unlock(b"missing"), Err(NotLocked));
    }

    #[test]
    fn wake_prefers_waiters_then_lockers_and_removes_empty() {
        let i = ids(3);
        let mut ch = WaitChannels::default();
        ch.lock(b"c", i[0]);
        ch.lock(b"c", i[1]);
        ch.wait(b"c", i[2]);
        assert_eq!(ch.wake(b"c", |x| x == i[1] || x == i[2]), resumed(&[i[2]]));
        assert_eq!(ch.wake(b"c", |x| x == i[1]), resumed(&[i[1]]));
        assert!(ch.channels.contains_key(b"c".as_slice()), "still locked");
        assert_eq!(ch.wake(b"c", |_| true), WaitOutcome::normal());
        assert_eq!(ch.wake(b"none", |_| true), WaitOutcome::normal());
        // remove_empty: unlocked, unwoken, both lists empty.
        let mut ch = WaitChannels::default();
        ch.wait(b"d", i[0]);
        assert_eq!(ch.wake(b"d", |_| true), resumed(&[i[0]]));
        assert!(ch.channels.is_empty());
    }

    #[test]
    fn list_order_is_waiters_then_lockers() {
        let i = ids(3);
        let mut ch = WaitChannels::default();
        ch.lock(b"c", i[0]);
        ch.lock(b"c", i[1]);
        ch.wait(b"c", i[2]);
        assert_eq!(ch.list(b"c").collect::<Vec<_>>(), vec![i[2], i[1]]);
        assert_eq!(ch.list(b"zz").count(), 0);
    }

    #[test]
    fn client_lost_and_flush() {
        let i = ids(4);
        let mut ch = WaitChannels::default();
        ch.lock(b"c", i[0]);
        ch.lock(b"c", i[1]);
        ch.wait(b"c", i[2]);
        ch.wait(b"e", i[3]);
        assert_eq!(ch.client_lost(|x| x == i[1] || x == i[3]), vec![i[1], i[3]]);
        assert!(!ch.channels.contains_key(b"e".as_slice()));
        assert!(ch.channels[b"c".as_slice()].locked);
        assert_eq!(ch.flush(), vec![i[2]]);
        assert!(ch.channels.is_empty());
        let _ = i[0].parts();
    }
}
