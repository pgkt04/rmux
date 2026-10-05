// Ported from tmux tty.c and screen-redraw.c @ 8f25579c
use super::*;
use crate::model::session::{SessionCreate, session_create};
use crate::model::spawn::{SpawnContext, SpawnFlags, spawn_window};
use crate::options::environment::Environment;
use rmux_emu::image::SixelImage;
use rmux_sys::OwnedFd;
use rmux_tty::tty::{Tty, TtyHostInfo, TtyOptions};
use rmux_util::bytes::ByteString;
use std::num::NonZeroU32;
use std::os::fd::AsFd;

fn fixture() -> (Server, ClientId, PaneId, Tty, TparmState, OwnedFd) {
    let mut srv = Server::new();
    let options = srv.options.create(Some(srv.options.global_s));
    let session = session_create(
        &mut srv,
        SessionCreate {
            prefix: None,
            name: Some(b"image-replay".to_vec()),
            cwd: b"/tmp".to_vec(),
            environment: Environment::new(),
            options,
            termios: None,
        },
    );
    let mut spawn = SpawnContext::new(session);
    spawn.flags = SpawnFlags::EMPTY;
    let link = spawn_window(&mut srv, &mut spawn).unwrap();
    srv.sessions.get_mut(session).unwrap().current = Some(link);
    let wp = spawn.pane.unwrap();
    let mut client = Client::new(None, (0, 0));
    client.session = Some(session);
    let c = srv.clients.insert(client).unwrap();
    let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
    let tio = rmux_sys::termios::TermiosState::get(slave.as_fd()).unwrap();
    let mut tty = Tty::new(
        slave,
        tio,
        TtyHostInfo {
            utf8: true,
            ..Default::default()
        },
    );
    let mut tparm = TparmState::default();
    tty.set_size(8, 4, 1, 1);
    let caps = [
        "clear=\x1b[H\x1b[2J",
        "cup=\x1b[%i%p1%d;%p2%dH",
        "csr=\x1b[%i%p1%d;%p2%dr",
        "sgr0=\x1b[0m",
        "am=1",
        "Sxl=1",
    ]
    .into_iter()
    .map(|s| ByteString(s.as_bytes().to_vec()))
    .collect();
    tty.open(
        &mut tparm,
        b"image-fixture",
        &caps,
        &TtyOptions::default(),
        None,
    )
    .unwrap();
    tty.set_window_offset(false, 0, 0, 8, 4);
    capture(&mut tty, &master);
    tty.flags_mut().remove(TtyFlags::STARTED);
    tty.invalidate(&mut tparm);
    (srv, c, wp, tty, tparm, master)
}

fn capture(tty: &mut Tty, master: &OwnedFd) -> Vec<u8> {
    let count = tty.out_len();
    let mut bytes = vec![0; count];
    let mut written = 0;
    while written < count {
        written += tty.on_writable().unwrap();
    }
    let mut received = 0;
    while received < count {
        received += rmux_sys::fd::read(master.as_fd(), &mut bytes[received..]).unwrap();
    }
    bytes
}

fn store(srv: &mut Server, wp: PaneId, register: u32) {
    let owner = srv.panes.get(wp).unwrap().screen().image_owner().unwrap();
    let payload = format!("q\"1;1;1;1#{register}@");
    let one = NonZeroU32::new(1).unwrap();
    let data = SixelImage::parse(payload.as_bytes(), 0, one, one).unwrap();
    srv.images.store(owner, data, 0, 0);
}

#[test]
fn replay_uses_insertion_order_and_current_alternate_owner() {
    let (mut srv, c, wp, mut tty, mut tparm, master) = fixture();
    store(&mut srv, wp, 1);
    store(&mut srv, wp, 0);
    tty_draw_images(&mut tty, &mut tparm, &srv, c, wp);
    assert_eq!(capture(&mut tty, &master), b"\x1b[1;1H\x1b[1;4r\x1b[1;1H\x1bP9;0q\"1;1;1;1#1@\x1b\\\x1b[1;1H\x1b[1;4r\x1b[1;1H\x1bP9;0q\"1;1;1;1#0@\x1b\\");
    srv.panes
        .get_mut(wp)
        .unwrap()
        .base
        .alternate_on(&DEFAULT_CELL, false, Some(&mut srv.images));
    tty_draw_images(&mut tty, &mut tparm, &srv, c, wp);
    assert!(capture(&mut tty, &master).is_empty());
    store(&mut srv, wp, 2);
    tty_draw_images(&mut tty, &mut tparm, &srv, c, wp);
    let alternate = capture(&mut tty, &master);
    assert!(alternate.ends_with(b"\x1bP9;0q\"1;1;1;1#2@\x1b\\"));
    srv.panes
        .get_mut(wp)
        .unwrap()
        .base
        .alternate_off(None, false, Some(&mut srv.images));
    tty_draw_images(&mut tty, &mut tparm, &srv, c, wp);
    let restored = capture(&mut tty, &master);
    assert!(restored.ends_with(b"\x1bP9;0q\"1;1;1;1#0@\x1b\\"));
    assert!(!restored.windows(3).any(|b| b == b"#2@"));
}

#[test]
fn replay_skips_nonselected_windows_and_panes_without_layout_cells() {
    let (mut srv, c, wp, mut tty, mut tparm, master) = fixture();
    store(&mut srv, wp, 0);
    let session = srv.clients.get(c).unwrap().session;
    srv.clients.get_mut(c).unwrap().session = None;
    tty_draw_images(&mut tty, &mut tparm, &srv, c, wp);
    assert!(capture(&mut tty, &master).is_empty());
    srv.clients.get_mut(c).unwrap().session = session;
    srv.panes.get_mut(wp).unwrap().layout_cell = None;
    tty_draw_images(&mut tty, &mut tparm, &srv, c, wp);
    assert!(capture(&mut tty, &master).is_empty());
}
