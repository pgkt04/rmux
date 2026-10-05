use std::collections::BTreeMap;

use super::*;
use crate::ids::Arena;

#[derive(Default)]
struct Fixture {
    sessions: Arena<FxSession, SessionId>,
    winlinks: Arena<FxWinlink, WinlinkId>,
    windows: Arena<FxWindow, WindowId>,
    panes: Arena<FxPane, PaneId>,
    buffers: Arena<FxBuffer, PasteBufferId>,
    clients: Arena<FxClient, ClientId>,
    session_names: BTreeMap<Vec<u8>, SessionId>,
    client_order: Vec<ClientId>,
    buffer_order: Vec<PasteBufferId>,
}

struct FxSession {
    public_id: u32,
    name: Vec<u8>,
    created: TimeVal,
    activity: TimeVal,
    windows: BTreeMap<i32, WinlinkId>,
}
struct FxWinlink {
    index: i32,
    window: WindowId,
}
struct FxWindow {
    name: Vec<u8>,
    created: TimeVal,
    activity: TimeVal,
    size: (u32, u32),
    panes: Vec<PaneId>,
}
struct FxPane {
    public_id: u32,
    active_point: u64,
    size: (u32, u32),
    index: u32,
    zindex: u32,
    title: Vec<u8>,
}
struct FxBuffer {
    name: Vec<u8>,
    order: u32,
    size: usize,
}
struct FxClient {
    name: Vec<u8>,
    sortable: bool,
    size: (u32, u32),
    created: TimeVal,
    activity: TimeVal,
}

impl SortModel for Fixture {
    fn sessions(&self, out: &mut Vec<SessionId>) {
        out.extend(self.session_names.values().copied());
    }
    fn winlinks(&self, session: SessionId, out: &mut Vec<WinlinkId>) {
        if let Some(s) = self.sessions.get(session) {
            out.extend(s.windows.values().copied());
        }
    }
    fn panes(&self, window: WindowId, out: &mut Vec<PaneId>) {
        if let Some(w) = self.windows.get(window) {
            out.extend_from_slice(&w.panes);
        }
    }
    fn winlink_window(&self, winlink: WinlinkId) -> Option<WindowId> {
        self.winlinks.get(winlink).map(|wl| wl.window)
    }
    fn buffers(&self, out: &mut Vec<PasteBufferId>) {
        out.extend_from_slice(&self.buffer_order);
    }
    fn session_public_id(&self, s: SessionId) -> u32 {
        self.sessions.get(s).map_or(0, |s| s.public_id)
    }
    fn session_created(&self, s: SessionId) -> TimeVal {
        self.sessions.get(s).map_or((0, 0), |s| s.created)
    }
    fn session_activity(&self, s: SessionId) -> TimeVal {
        self.sessions.get(s).map_or((0, 0), |s| s.activity)
    }
    fn session_name(&self, s: SessionId) -> &[u8] {
        self.sessions.get(s).map_or(&[], |s| &s.name)
    }
    fn winlink_index(&self, wl: WinlinkId) -> i32 {
        self.winlinks.get(wl).map_or(0, |wl| wl.index)
    }
    fn window_created(&self, w: WindowId) -> TimeVal {
        self.windows.get(w).map_or((0, 0), |w| w.created)
    }
    fn window_activity(&self, w: WindowId) -> TimeVal {
        self.windows.get(w).map_or((0, 0), |w| w.activity)
    }
    fn window_name(&self, w: WindowId) -> &[u8] {
        self.windows.get(w).map_or(&[], |w| &w.name)
    }
    fn window_size(&self, w: WindowId) -> (u32, u32) {
        self.windows.get(w).map_or((0, 0), |w| w.size)
    }
    fn pane_active_point(&self, p: PaneId) -> u64 {
        self.panes.get(p).map_or(0, |p| p.active_point)
    }
    fn pane_public_id(&self, p: PaneId) -> u32 {
        self.panes.get(p).map_or(0, |p| p.public_id)
    }
    fn pane_size(&self, p: PaneId) -> (u32, u32) {
        self.panes.get(p).map_or((0, 0), |p| p.size)
    }
    fn pane_index(&self, p: PaneId) -> u32 {
        self.panes.get(p).map_or(0, |p| p.index)
    }
    fn pane_zindex(&self, p: PaneId) -> u32 {
        self.panes.get(p).map_or(0, |p| p.zindex)
    }
    fn pane_title(&self, p: PaneId) -> &[u8] {
        self.panes.get(p).map_or(&[], |p| &p.title)
    }
    fn buffer_name(&self, b: PasteBufferId) -> &[u8] {
        self.buffers.get(b).map_or(&[], |b| &b.name)
    }
    fn buffer_order(&self, b: PasteBufferId) -> u32 {
        self.buffers.get(b).map_or(0, |b| b.order)
    }
    fn buffer_size(&self, b: PasteBufferId) -> usize {
        self.buffers.get(b).map_or(0, |b| b.size)
    }
}

impl SortClients for Fixture {
    fn clients(&self, out: &mut Vec<ClientId>) {
        out.extend_from_slice(&self.client_order);
    }
    fn client_sortable(&self, c: ClientId) -> bool {
        self.clients.get(c).is_some_and(|c| c.sortable)
    }
    fn client_name(&self, c: ClientId) -> &[u8] {
        self.clients.get(c).map_or(&[], |c| &c.name)
    }
    fn client_size(&self, c: ClientId) -> (u32, u32) {
        self.clients.get(c).map_or((0, 0), |c| c.size)
    }
    fn client_created(&self, c: ClientId) -> TimeVal {
        self.clients.get(c).map_or((0, 0), |c| c.created)
    }
    fn client_activity(&self, c: ClientId) -> TimeVal {
        self.clients.get(c).map_or((0, 0), |c| c.activity)
    }
}

impl Fixture {
    fn session(&mut self, name: &str, public_id: u32, created: i64, activity: i64) -> SessionId {
        let id = self
            .sessions
            .insert(FxSession {
                public_id,
                name: name.into(),
                created: (created, 0),
                activity: (activity, 0),
                windows: BTreeMap::new(),
            })
            .unwrap();
        self.session_names.insert(name.into(), id);
        id
    }
    fn window(&mut self, name: &str, created: i64, activity: i64, size: (u32, u32)) -> WindowId {
        self.windows
            .insert(FxWindow {
                name: name.into(),
                created: (created, 0),
                activity: (activity, 0),
                size,
                panes: Vec::new(),
            })
            .unwrap()
    }
    fn link(&mut self, session: SessionId, index: i32, window: WindowId) -> WinlinkId {
        let id = self.winlinks.insert(FxWinlink { index, window }).unwrap();
        self.sessions
            .get_mut(session)
            .unwrap()
            .windows
            .insert(index, id);
        id
    }
    fn pane(
        &mut self,
        window: WindowId,
        public_id: u32,
        active_point: u64,
        size: (u32, u32),
        zindex: u32,
        title: &str,
    ) -> PaneId {
        let index = self.windows.get(window).unwrap().panes.len() as u32;
        let id = self
            .panes
            .insert(FxPane {
                public_id,
                active_point,
                size,
                index,
                zindex,
                title: title.into(),
            })
            .unwrap();
        self.windows.get_mut(window).unwrap().panes.push(id);
        id
    }
    fn buffer(&mut self, name: &str, order: u32, size: usize) -> PasteBufferId {
        let id = self
            .buffers
            .insert(FxBuffer {
                name: name.into(),
                order,
                size,
            })
            .unwrap();
        self.buffer_order.push(id);
        id
    }
    fn client(
        &mut self,
        name: &str,
        sortable: bool,
        size: (u32, u32),
        created: i64,
        activity: i64,
    ) -> ClientId {
        let id = self
            .clients
            .insert(FxClient {
                name: name.into(),
                sortable,
                size,
                created: (created, 0),
                activity: (activity, 0),
            })
            .unwrap();
        self.client_order.push(id);
        id
    }
}

fn crit(order: SortOrder, reversed: bool) -> SortCriteria {
    SortCriteria::new(order, reversed)
}

#[test]
fn order_names_round_trip() {
    for (name, order) in [
        ("activity", SortOrder::Activity),
        ("CREATION", SortOrder::Creation),
        ("index", SortOrder::Index),
        ("Key", SortOrder::Index),
        ("modifier", SortOrder::Modifier),
        ("name", SortOrder::Name),
        ("title", SortOrder::Name),
        ("order", SortOrder::Order),
        ("size", SortOrder::Size),
        ("Z", SortOrder::Z),
        ("bogus", SortOrder::End),
        ("", SortOrder::End),
    ] {
        assert_eq!(order_from_string(Some(name.as_bytes())), order, "{name}");
    }
    assert_eq!(order_from_string(None), SortOrder::End);
    assert_eq!(order_to_string(SortOrder::Name), Some(&b"name"[..]));
    assert_eq!(order_to_string(SortOrder::End), None);
    for order in [
        SortOrder::Activity,
        SortOrder::Creation,
        SortOrder::Index,
        SortOrder::Modifier,
        SortOrder::Name,
        SortOrder::Order,
        SortOrder::Size,
        SortOrder::Z,
    ] {
        assert_eq!(order_from_string(order_to_string(order)), order);
    }
}

#[test]
fn next_order_wraps_and_handles_absent_order() {
    static SEQ: [SortOrder; 4] = [
        SortOrder::Index,
        SortOrder::Name,
        SortOrder::Activity,
        SortOrder::End,
    ];
    let mut c = SortCriteria {
        order: SortOrder::Index,
        reversed: false,
        order_seq: Some(&SEQ),
    };
    next_order(&mut c);
    assert_eq!(c.order, SortOrder::Name);
    next_order(&mut c);
    assert_eq!(c.order, SortOrder::Activity);
    next_order(&mut c);
    assert_eq!(c.order, SortOrder::Index);
    c.order = SortOrder::Size;
    next_order(&mut c);
    assert_eq!(c.order, SortOrder::Index);

    static UNTERMINATED: [SortOrder; 2] = [SortOrder::Name, SortOrder::Size];
    c.order_seq = Some(&UNTERMINATED);
    c.order = SortOrder::Size;
    next_order(&mut c);
    assert_eq!(c.order, SortOrder::Name);

    let mut none = crit(SortOrder::Size, true);
    next_order(&mut none);
    assert_eq!(none.order, SortOrder::Size);
}

#[test]
fn sessions_sort_by_every_order_with_name_tie_break() {
    let mut fx = Fixture::default();
    let b = fx.session("b", 2, 10, 50);
    let a = fx.session("a", 3, 20, 50);
    let c = fx.session("c", 1, 20, 90);
    let mut out = Vec::new();

    get_sessions(&fx, &crit(SortOrder::Index, false), &mut out);
    assert_eq!(out, [c, b, a]);
    get_sessions(&fx, &crit(SortOrder::Index, true), &mut out);
    assert_eq!(out, [a, b, c]);
    get_sessions(&fx, &crit(SortOrder::Name, false), &mut out);
    assert_eq!(out, [a, b, c]);
    get_sessions(&fx, &crit(SortOrder::Creation, false), &mut out);
    assert_eq!(out, [b, a, c]);
    get_sessions(&fx, &crit(SortOrder::Activity, false), &mut out);
    assert_eq!(out, [c, a, b]);
    get_sessions(&fx, &crit(SortOrder::Activity, true), &mut out);
    assert_eq!(out, [b, a, c]);
    get_sessions(&fx, &crit(SortOrder::Size, false), &mut out);
    assert_eq!(out, [a, b, c], "unsupported order falls to name");
    get_sessions(&fx, &crit(SortOrder::Order, false), &mut out);
    assert_eq!(out, [a, b, c], "collection order kept");
    get_sessions(&fx, &crit(SortOrder::Order, true), &mut out);
    assert_eq!(
        out,
        [c, b, a],
        "collection order reversed without comparing"
    );
    get_sessions(&fx, &crit(SortOrder::End, true), &mut out);
    assert_eq!(out, [a, b, c], "End never sorts");
}

#[test]
fn winlinks_panes_and_tree_swap() {
    let mut fx = Fixture::default();
    let s1 = fx.session("one", 0, 0, 0);
    let s2 = fx.session("two", 1, 0, 0);
    let w_shell = fx.window("shell", 5, 30, (80, 24));
    let w_vim = fx.window("vim", 1, 40, (80, 10));
    let w_zsh = fx.window("zsh", 3, 40, (10, 10));
    let l0 = fx.link(s1, 0, w_vim);
    let l1 = fx.link(s1, 1, w_shell);
    let l2 = fx.link(s1, 2, w_zsh);
    let l9 = fx.link(s2, 9, w_vim);
    let p_a = fx.pane(w_vim, 7, 3, (40, 10), 2, "b");
    let p_b = fx.pane(w_vim, 2, 9, (40, 10), 1, "a");
    let p_c = fx.pane(w_shell, 5, 1, (80, 24), 0, "c");

    let mut wl = Vec::new();
    get_winlinks_session(&fx, s1, &crit(SortOrder::Index, false), &mut wl);
    assert_eq!(wl, [l0, l1, l2]);
    get_winlinks_session(&fx, s1, &crit(SortOrder::Name, true), &mut wl);
    assert_eq!(wl, [l2, l0, l1]);
    get_winlinks_session(&fx, s1, &crit(SortOrder::Creation, false), &mut wl);
    assert_eq!(wl, [l0, l2, l1]);
    get_winlinks_session(&fx, s1, &crit(SortOrder::Activity, false), &mut wl);
    assert_eq!(wl, [l0, l2, l1], "activity tie falls to window name");
    get_winlinks_session(&fx, s1, &crit(SortOrder::Size, false), &mut wl);
    assert_eq!(wl, [l2, l0, l1]);
    get_winlinks(&fx, &crit(SortOrder::Index, false), &mut wl);
    assert_eq!(wl, [l0, l1, l2, l9]);

    let c = crit(SortOrder::Name, false);
    assert!(!would_window_tree_swap(
        &fx,
        &crit(SortOrder::Index, false),
        l0,
        l1
    ));
    assert!(would_window_tree_swap(&fx, &c, l0, l1));
    assert!(
        !would_window_tree_swap(&fx, &c, l0, l9),
        "same window name compares equal"
    );

    let mut panes = Vec::new();
    get_panes_window(&fx, w_vim, &crit(SortOrder::Creation, false), &mut panes);
    assert_eq!(panes, [p_b, p_a]);
    get_panes_window(&fx, w_vim, &crit(SortOrder::Activity, false), &mut panes);
    assert_eq!(panes, [p_a, p_b]);
    get_panes_window(&fx, w_vim, &crit(SortOrder::Index, false), &mut panes);
    assert_eq!(panes, [p_a, p_b]);
    get_panes_window(&fx, w_vim, &crit(SortOrder::Z, false), &mut panes);
    assert_eq!(panes, [p_b, p_a]);
    get_panes_window(&fx, w_vim, &crit(SortOrder::Name, false), &mut panes);
    assert_eq!(panes, [p_b, p_a]);
    get_panes_window(&fx, w_vim, &crit(SortOrder::Size, false), &mut panes);
    assert_eq!(panes, [p_b, p_a], "size tie falls to title");
    get_panes_session(&fx, s1, &crit(SortOrder::Order, false), &mut panes);
    assert_eq!(panes, [p_a, p_b, p_c]);
    get_panes(&fx, &crit(SortOrder::Order, false), &mut panes);
    assert_eq!(
        panes,
        [p_a, p_b, p_c, p_a, p_b],
        "linked window repeats panes"
    );
    get_panes(&fx, &crit(SortOrder::Creation, true), &mut panes);
    assert_eq!(panes, [p_a, p_a, p_c, p_b, p_b]);
}

#[test]
fn buffers_and_clients() {
    let mut fx = Fixture::default();
    let b0 = fx.buffer("buffer0", 0, 5);
    let b1 = fx.buffer("buffer1", 1, 3);
    let b2 = fx.buffer("abc", 2, 3);
    let mut out = Vec::new();
    get_buffers(&fx, &crit(SortOrder::Name, false), &mut out);
    assert_eq!(out, [b2, b0, b1]);
    get_buffers(&fx, &crit(SortOrder::Creation, false), &mut out);
    assert_eq!(out, [b2, b1, b0], "creation is order descending");
    get_buffers(&fx, &crit(SortOrder::Size, false), &mut out);
    assert_eq!(out, [b2, b1, b0]);
    get_buffers(&fx, &crit(SortOrder::Size, true), &mut out);
    assert_eq!(out, [b0, b1, b2], "reverse includes the name tie key");

    let c1 = fx.client("/dev/ttys001", true, (80, 24), 10, 5);
    let _c2 = fx.client("/dev/ttys002", false, (80, 24), 11, 9);
    let c3 = fx.client("/dev/ttys000", true, (80, 50), 12, 9);
    let c4 = fx.client("/dev/ttys003", true, (40, 24), 12, 1);
    let mut cl = Vec::new();
    get_clients(&fx, &crit(SortOrder::Order, false), &mut cl);
    assert_eq!(cl, [c1, c3, c4], "unattached clients are excluded");
    get_clients(&fx, &crit(SortOrder::Name, false), &mut cl);
    assert_eq!(cl, [c3, c1, c4]);
    get_clients(&fx, &crit(SortOrder::Size, false), &mut cl);
    assert_eq!(cl, [c4, c1, c3]);
    get_clients(&fx, &crit(SortOrder::Creation, false), &mut cl);
    assert_eq!(cl, [c1, c3, c4], "creation tie falls to name");
    get_clients(&fx, &crit(SortOrder::Activity, false), &mut cl);
    assert_eq!(cl, [c3, c1, c4]);
}

#[test]
fn key_bindings_sort_by_key_modifier_and_table() {
    use crate::cmd::CommandList;
    use rmux_util::key::KeyModifiers;
    use std::rc::Rc;
    let mut kb = KeyBindings::new();
    let list = Rc::new(CommandList::default());
    let add = |kb: &mut KeyBindings, table: &str, key: u64| {
        kb.add(
            table.as_bytes(),
            KeyCode(key),
            None,
            false,
            Some(Rc::clone(&list)),
        )
        .unwrap();
    };
    let meta = KeyModifiers::META.0;
    let ctrl = KeyModifiers::CTRL.0;
    add(&mut kb, "root", b'a' as u64 | ctrl);
    add(&mut kb, "prefix", b'c' as u64);
    add(&mut kb, "Prefix", b'b' as u64 | meta);
    add(&mut kb, "root", b'd' as u64);
    let root = kb.find_table(b"root").unwrap();
    let prefix = kb.find_table(b"prefix").unwrap();
    let cap = kb.find_table(b"Prefix").unwrap();

    let mut out = Vec::new();
    get_key_bindings(&kb, &crit(SortOrder::Order, false), &mut out);
    assert_eq!(
        out,
        [
            (cap, KeyCode(b'b' as u64 | meta)),
            (prefix, KeyCode(b'c' as u64)),
            (root, KeyCode(b'd' as u64)),
            (root, KeyCode(b'a' as u64 | ctrl)),
        ],
        "tables in byte order, keys in key order"
    );
    get_key_bindings(&kb, &crit(SortOrder::Index, false), &mut out);
    assert_eq!(
        out,
        [
            (prefix, KeyCode(b'c' as u64)),
            (root, KeyCode(b'd' as u64)),
            (cap, KeyCode(b'b' as u64 | meta)),
            (root, KeyCode(b'a' as u64 | ctrl)),
        ]
    );
    get_key_bindings(&kb, &crit(SortOrder::Modifier, false), &mut out);
    assert_eq!(
        out,
        [
            (prefix, KeyCode(b'c' as u64)),
            (root, KeyCode(b'd' as u64)),
            (cap, KeyCode(b'b' as u64 | meta)),
            (root, KeyCode(b'a' as u64 | ctrl)),
        ]
    );
    get_key_bindings(&kb, &crit(SortOrder::Name, false), &mut out);
    assert_eq!(
        out,
        [
            (prefix, KeyCode(b'c' as u64)),
            (cap, KeyCode(b'b' as u64 | meta)),
            (root, KeyCode(b'd' as u64)),
            (root, KeyCode(b'a' as u64 | ctrl)),
        ],
        "case-insensitive table name, then key"
    );
    get_key_bindings_table(&kb, root, &crit(SortOrder::Index, true), &mut out);
    assert_eq!(
        out,
        [
            (root, KeyCode(b'a' as u64 | ctrl)),
            (root, KeyCode(b'd' as u64)),
        ]
    );
}

#[test]
fn sort_names_end_at_first_nul() {
    assert_eq!(order_from_string(Some(b"NAME\0unknown")), SortOrder::Name);
    assert_eq!(strcmp(b"a\0z", b"a\0b"), Ordering::Equal);
    assert_eq!(strcasecmp(b"Prefix\0z", b"prefix\0b"), Ordering::Equal);
    let mut fx = Fixture::default();
    let a = fx.buffer("a", 2, 10);
    let b = fx.buffer("a", 2, 10);
    fx.buffers.get_mut(a).unwrap().name = b"a\0z".to_vec();
    fx.buffers.get_mut(b).unwrap().name = b"a\0b".to_vec();
    assert_eq!(
        buffer_cmp(&fx, &crit(SortOrder::Name, false), a, b),
        Ordering::Equal
    );
}

struct PinnedFixture {
    facts: Fixture,
    bindings: KeyBindings,
    buffers: [PasteBufferId; 4],
    clients: [ClientId; 4],
    sessions: [SessionId; 4],
    windows: [WindowId; 3],
    panes: [PaneId; 4],
    winlinks: [WinlinkId; 4],
    keys: [(KeyTableId, KeyCode); 4],
}

impl PinnedFixture {
    fn new() -> Self {
        use crate::cmd::CommandList;
        use rmux_util::key::KeyModifiers;
        use std::rc::Rc;

        let names = ["z", "a", "A", "a"];
        let ids = [9, 2, 7, 4];
        let sizes = [(65536, 65536), (80, 24), (80, 10), (40, 48)];
        let seconds = [20, 10, 20, 10];
        let micros = [2, 9, 1, 9];
        let mut facts = Fixture::default();
        let sessions = std::array::from_fn(|i| {
            let session = facts.session(
                if i == 3 { "b" } else { names[i] },
                ids[i],
                seconds[i],
                seconds[3 - i],
            );
            let s = facts.sessions.get_mut(session).unwrap();
            s.created.1 = micros[i];
            s.activity.1 = micros[i];
            session
        });
        let clients = std::array::from_fn(|i| {
            let client = facts.client(
                names[i],
                i == 0 || i == 3,
                sizes[i],
                seconds[i],
                seconds[3 - i],
            );
            let c = facts.clients.get_mut(client).unwrap();
            c.created.1 = micros[i];
            c.activity.1 = micros[i];
            client
        });
        let buffers = std::array::from_fn(|i| {
            facts.buffer(
                if i == 3 { "b" } else { names[i] },
                ids[i],
                if i == 0 { 100 } else { 3 },
            )
        });
        facts.buffer_order = vec![buffers[0], buffers[2], buffers[3], buffers[1]];
        let windows = std::array::from_fn(|i| {
            let window = facts.window(names[i], seconds[i], seconds[3 - i], sizes[i]);
            let w = facts.windows.get_mut(window).unwrap();
            w.created.1 = micros[i];
            w.activity.1 = micros[i];
            window
        });
        let pane_windows = [0, 0, 1, 2];
        let active_points = [8, 1, 8, 3];
        let zindexes = [1, 0, 1, 1];
        let panes = std::array::from_fn(|i| {
            facts.pane(
                windows[pane_windows[i]],
                ids[i],
                active_points[i],
                sizes[i],
                zindexes[i],
                names[i],
            )
        });
        let link_windows = [0, 1, 0, 2];
        let indexes = [3, -2, 7, 0];
        let winlinks = std::array::from_fn(|i| {
            facts.link(
                sessions[usize::from(i >= 3)],
                indexes[i],
                windows[link_windows[i]],
            )
        });
        let mut bindings = KeyBindings::new();
        let tables = ["root", "prefix", "Prefix", "root"];
        let codes = [
            KeyCode(u64::from(b'a') | KeyModifiers::CTRL.0),
            KeyCode(u64::from(b'c')),
            KeyCode(u64::from(b'b') | KeyModifiers::META.0),
            KeyCode(u64::from(b'd')),
        ];
        let list = Rc::new(CommandList::default());
        let keys = std::array::from_fn(|i| {
            bindings
                .add(
                    tables[i].as_bytes(),
                    codes[i],
                    None,
                    false,
                    Some(Rc::clone(&list)),
                )
                .unwrap();
            (bindings.find_table(tables[i].as_bytes()).unwrap(), codes[i])
        });
        Self {
            facts,
            bindings,
            buffers,
            clients,
            sessions,
            windows,
            panes,
            winlinks,
            keys,
        }
    }

    fn compare(&self, kind: usize, criteria: &SortCriteria, a: usize, b: usize) -> Ordering {
        match kind {
            0 => buffer_cmp(&self.facts, criteria, self.buffers[a], self.buffers[b]),
            1 => client_cmp(&self.facts, criteria, self.clients[a], self.clients[b]),
            2 => session_cmp(&self.facts, criteria, self.sessions[a], self.sessions[b]),
            3 => pane_cmp(&self.facts, criteria, self.panes[a], self.panes[b]),
            4 => winlink_cmp(&self.facts, criteria, self.winlinks[a], self.winlinks[b]),
            5 => key_binding_cmp(&self.bindings, criteria, self.keys[a], self.keys[b]),
            _ => unreachable!(),
        }
    }

    fn collect(&self, kind: usize, criteria: &SortCriteria, scope: usize) -> Vec<usize> {
        fn indices<T: PartialEq>(out: Vec<T>, ids: &[T]) -> Vec<usize> {
            out.iter()
                .map(|id| ids.iter().position(|other| id == other).unwrap())
                .collect()
        }
        match kind {
            0 => {
                let mut out = vec![self.buffers[1]];
                get_buffers(&self.facts, criteria, &mut out);
                indices(out, &self.buffers)
            }
            1 => {
                let mut out = vec![self.clients[1]];
                get_clients(&self.facts, criteria, &mut out);
                indices(out, &self.clients)
            }
            2 => {
                let mut out = vec![self.sessions[1]];
                get_sessions(&self.facts, criteria, &mut out);
                indices(out, &self.sessions)
            }
            3 => {
                let mut out = vec![self.panes[3]];
                match scope {
                    0 => get_panes(&self.facts, criteria, &mut out),
                    1 => get_panes_session(&self.facts, self.sessions[0], criteria, &mut out),
                    _ => get_panes_window(&self.facts, self.windows[0], criteria, &mut out),
                }
                indices(out, &self.panes)
            }
            4 => {
                let mut out = vec![self.winlinks[3]];
                if scope == 0 {
                    get_winlinks(&self.facts, criteria, &mut out);
                } else {
                    get_winlinks_session(&self.facts, self.sessions[0], criteria, &mut out);
                }
                indices(out, &self.winlinks)
            }
            5 => {
                let mut out = vec![self.keys[1]];
                if scope == 0 {
                    get_key_bindings(&self.bindings, criteria, &mut out);
                } else {
                    get_key_bindings_table(&self.bindings, self.keys[0].0, criteria, &mut out);
                }
                indices(out, &self.keys)
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn pinned_fixture_ties_overflow_microseconds_and_collectors() {
    let mut fixture = PinnedFixture::new();
    let c = crit(SortOrder::Creation, false);
    assert_eq!(
        fixture.compare(2, &c, 3, 1),
        Ordering::Greater,
        "time tie uses name"
    );
    assert_eq!(
        fixture.compare(2, &c, 2, 0),
        Ordering::Less,
        "microseconds precede name"
    );
    assert_eq!(
        fixture.compare(3, &crit(SortOrder::Size, false), 0, 1),
        Ordering::Less,
        "C unsigned area wraps"
    );
    assert_eq!(
        fixture.compare(4, &crit(SortOrder::Name, true), 0, 2),
        Ordering::Equal,
        "linked-window tie"
    );
    assert_eq!(
        fixture.collect(3, &crit(SortOrder::Order, false), 0),
        [3, 2, 0, 1, 0, 1]
    );
    assert_eq!(
        fixture.collect(3, &crit(SortOrder::End, true), 1),
        [2, 0, 1, 0, 1]
    );
    assert_eq!(fixture.collect(3, &crit(SortOrder::Order, true), 2), [1, 0]);
    assert_eq!(
        fixture.collect(1, &crit(SortOrder::Order, false), 0),
        [0, 3]
    );
    assert_eq!(
        fixture.collect(0, &crit(SortOrder::Order, false), 0),
        [0, 2, 3, 1]
    );
    assert_eq!(
        fixture.collect(5, &crit(SortOrder::Order, false), 0),
        [2, 1, 3, 0]
    );
    assert_eq!(
        fixture.collect(4, &crit(SortOrder::Name, false), 0),
        [3, 1, 0, 2],
        "pinned macOS qsort retains tied winlinks"
    );
    let mut panes = vec![fixture.panes[0]];
    get_panes_window(
        &fixture.facts,
        fixture.windows[0],
        &crit(SortOrder::Order, false),
        &mut panes,
    );
    fixture
        .facts
        .windows
        .get_mut(fixture.windows[0])
        .unwrap()
        .panes
        .clear();
    get_panes_window(
        &fixture.facts,
        fixture.windows[0],
        &crit(SortOrder::Order, false),
        &mut panes,
    );
    assert!(
        panes.is_empty(),
        "collector clears reused output when empty"
    );
}

#[test]
fn pinned_c_comparator_matrix_and_all_collectors() {
    use std::fmt::Write as _;
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let driver = std::env::var_os("RMUX_G10_SORT_DRIVER")
        .unwrap_or_else(|| "/tmp/swarm-rmux-build/AuxFuzzySort/sort-driver".into());
    if !std::path::Path::new(&driver).is_file() {
        eprintln!("SKIP sort pinned C comparison: set RMUX_G10_SORT_DRIVER");
        return;
    }
    let fixture = PinnedFixture::new();
    let mut input = String::new();
    let mut expected = Vec::new();
    for kind in 0..6 {
        for order in 0..=8 {
            for reversed in [false, true] {
                let criteria = crit(SortOrder::try_from(order).unwrap(), reversed);
                for a in 0..4 {
                    for b in 0..4 {
                        let request = format!("cmp {kind} {order} {} {a} {b}", u8::from(reversed));
                        writeln!(input, "{request}").unwrap();
                        let result = match fixture.compare(kind, &criteria, a, b) {
                            Ordering::Less => "-1",
                            Ordering::Equal => "0",
                            Ordering::Greater => "1",
                        };
                        expected.push((request, result.to_owned()));
                    }
                }
            }
        }
    }
    for kind in 0..6 {
        for order in 0..=8 {
            for reversed in [false, true] {
                let criteria = crit(SortOrder::try_from(order).unwrap(), reversed);
                let scopes = if kind == 3 {
                    3
                } else if kind >= 4 {
                    2
                } else {
                    1
                };
                for scope in 0..scopes {
                    let request =
                        format!("collect {kind} {order} {} {scope} 0", u8::from(reversed));
                    writeln!(input, "{request}").unwrap();
                    let result = fixture
                        .collect(kind, &criteria, scope)
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    expected.push((request, result));
                }
            }
        }
    }
    let mut child = Command::new(driver)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start pinned sort reference");
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reference = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = reference.lines().collect();
    assert_eq!(lines.len(), expected.len(), "reference row count");
    for ((request, actual), reference) in expected.iter().zip(lines) {
        assert_eq!(actual, reference, "{request}");
    }
}
