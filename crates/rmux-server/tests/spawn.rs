// Ported from tmux spawn.c @ 8f25579c
use rmux_server::model::session::{SessionCreate, session_create};
use rmux_server::model::spawn::*;
use rmux_server::model::{ModelEffect, PaneFlags, Server};
use rmux_server::options::environment::Environment;
use std::cell::RefCell;
use std::rc::Rc;

fn fixture() -> (Server, SpawnContext) {
    let mut server = Server::new();
    let options = server.options.create(Some(server.options.global_s));
    let session = session_create(
        &mut server,
        SessionCreate {
            prefix: None,
            name: Some(b"spawn-test".to_vec()),
            cwd: b"/tmp".to_vec(),
            environment: Environment::new(),
            options,
            termios: None,
        },
    );
    (server, SpawnContext::new(session))
}

#[test]
fn empty_spawn_and_respawn_preserve_identity_command_and_cwd() {
    let (mut server, mut context) = fixture();
    context.flags = SpawnFlags::EMPTY;
    context.argv = vec![b"printf first".to_vec()];
    context.cwd = Some(b"relative".to_vec());
    let link = spawn_window(&mut server, &mut context).unwrap();
    let pane = context.pane.unwrap();
    let p = server.panes.get(pane).unwrap();
    let public_id = p.public_id;
    assert_eq!(p.cwd, b"/tmp/relative");
    assert!(p.fd.is_none());
    assert!(p.flags.contains(PaneFlags::EMPTY));
    assert!(p.base.mode.contains(rmux_emu::screen::ScreenMode::CRLF));
    assert!(!p.base.mode.contains(rmux_emu::screen::ScreenMode::CURSOR));
    let event = server
        .effects
        .iter()
        .position(|e| matches!(e, ModelEffect::Event{name,..} if name == b"pane-created"))
        .unwrap();
    let window_event = server
        .effects
        .iter()
        .position(|e| matches!(e, ModelEffect::Event{name,..} if name == b"window-created"))
        .unwrap();
    assert!(event < window_event);
    context.flags = SpawnFlags::EMPTY | SpawnFlags::RESPAWN;
    context.argv.clear();
    context.cwd = None;
    context.winlink = Some(link);
    let returned = spawn_pane(&mut server, &mut context).unwrap();
    assert_eq!(returned, pane);
    let p = server.panes.get(pane).unwrap();
    assert_eq!(p.public_id, public_id);
    assert_eq!(p.argv, vec![b"printf first".to_vec()]);
    assert_eq!(p.cwd, b"/tmp/relative");
}

#[test]
fn modal_policy_and_occupied_index_error_are_exact() {
    let (mut server, mut context) = fixture();
    context.flags = SpawnFlags::EMPTY;
    context.index = 2;
    spawn_window(&mut server, &mut context).unwrap();
    let mut same = SpawnContext::new(context.session);
    same.flags = SpawnFlags::EMPTY;
    same.index = 2;
    assert_eq!(
        spawn_window(&mut server, &mut same)
            .unwrap_err()
            .to_string(),
        "index 2 in use"
    );
    context.flags = SpawnFlags::EMPTY | SpawnFlags::MODAL;
    assert_eq!(
        spawn_pane(&mut server, &mut context)
            .unwrap_err()
            .to_string(),
        "modal pane must be floating"
    );
}

struct StringOnlyParser;
impl rmux_server::cmd::parse::CommandParser for StringOnlyParser {
    fn parse_from_string(&mut self, _: &[u8]) -> rmux_server::cmd::parse::CmdParseResult {
        panic!("string options must not invoke command parsing")
    }
}

#[test]
fn new_pane_uses_default_command_and_invalid_shell_falls_back() {
    let (mut server, mut context) = fixture();
    let options = server.sessions.get(context.session).unwrap().options;
    let mut parser = StringOnlyParser;
    server.options.set_string(
        options,
        b"default-command",
        false,
        b"printf default",
        &mut parser,
    );
    server.options.set_string(
        options,
        b"default-shell",
        false,
        b"/rmux/no-such-shell",
        &mut parser,
    );
    context.flags = SpawnFlags::EMPTY;
    spawn_window(&mut server, &mut context).unwrap();
    let p = server.panes.get(context.pane.unwrap()).unwrap();
    assert_eq!(p.shell, b"/bin/sh");
    assert_eq!(p.argv, vec![b"printf default".to_vec()]);
}

#[test]
fn editor_finish_delivers_binary_data_once_and_cancel_still_unlinks() {
    let (mut server, mut context) = fixture();
    context.flags = SpawnFlags::EMPTY;
    spawn_window(&mut server, &mut context).unwrap();
    let pane = context.pane.unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    for (cancel, contents, status, expected) in [
        (
            false,
            Some(b"edited\0bytes".as_slice()),
            0,
            Some(b"edited\0bytes".to_vec()),
        ),
        (false, Some(b"".as_slice()), 0, None),
        (false, None, 0, None),
        (false, Some(b"ignored".as_slice()), 7 << 8, None),
        (true, Some(b"cancelled".as_slice()), 0, None),
    ] {
        let (file, path) =
            rmux_sys::proc::temporary_file(b"/tmp/rmux-editor-test.XXXXXXXX").unwrap();
        drop(file);
        if let Some(contents) = contents {
            std::fs::write(
                std::ffi::OsStr::new(std::str::from_utf8(&path).unwrap()),
                contents,
            )
            .unwrap();
        } else {
            std::fs::remove_file(std::str::from_utf8(&path).unwrap()).unwrap();
        }
        let capture = calls.clone();
        let id = server
            .editors
            .insert(SpawnEditorState {
                path: path.clone(),
                pid: rmux_sys::ProcessId(123),
                pane,
                callback: Some(Box::new(move |_, id, data| {
                    capture.borrow_mut().push((id, data))
                })),
            })
            .unwrap();
        let before = calls.borrow().len();
        let p = server.panes.get_mut(pane).unwrap();
        p.editor = Some(id);
        p.status = status;
        p.flags.insert(PaneFlags::STATUSREADY);
        if cancel {
            spawn_cancel_editor(&mut server, id);
        }
        spawn_editor_finish(&mut server, pane);
        spawn_editor_finish(&mut server, pane);
        assert!(!std::path::Path::new(std::str::from_utf8(&path).unwrap()).exists());
        assert!(server.editors.get(id).is_none());
        if cancel {
            assert_eq!(calls.borrow().len(), before);
        } else {
            assert_eq!(calls.borrow().len(), before + 1);
            assert_eq!(calls.borrow().last().unwrap().1, expected);
        }
    }
}

#[test]
fn zero_length_initial_editor_input_fails_without_a_pane() {
    let (mut server, mut context) = fixture();
    context.flags = SpawnFlags::EMPTY;
    spawn_window(&mut server, &mut context).unwrap();
    let before = server.pane_ids.len();
    assert!(
        spawn_editor(
            &mut server,
            &context,
            b"",
            Box::new(|_, _, _| panic!("failed editor callback"))
        )
        .is_err()
    );
    assert_eq!(server.pane_ids.len(), before);
}

#[test]
fn live_respawn_requires_kill_and_failure_retains_only_respawn_object() {
    let (mut server, mut context) = fixture();
    context.flags = SpawnFlags::EMPTY;
    spawn_window(&mut server, &mut context).unwrap();
    let pane = context.pane.unwrap();
    let (master, slave, _) = rmux_sys::pty::openpty().unwrap();
    server.panes.get_mut(pane).unwrap().fd = Some(master);
    context.flags = SpawnFlags::RESPAWN | SpawnFlags::EMPTY;
    assert_eq!(
        spawn_pane(&mut server, &mut context)
            .unwrap_err()
            .to_string(),
        "pane spawn-test:0.0 still active"
    );
    assert_eq!(
        spawn_window(&mut server, &mut context)
            .unwrap_err()
            .to_string(),
        "window spawn-test:0 still active"
    );
    context.flags = SpawnFlags::RESPAWN | SpawnFlags::KILL;
    context.argv = vec![b"invalid\0command".to_vec()];
    assert!(spawn_pane(&mut server, &mut context).is_err());
    assert!(server.panes.get(pane).unwrap().fd.is_none());
    assert!(server.pane_ids.values().any(|&id| id == pane));
    drop(slave);
    let mut ordinary = SpawnContext::new(context.session);
    ordinary.winlink = context.winlink;
    ordinary.argv = vec![b"invalid\0command".to_vec()];
    let before = server.pane_ids.len();
    assert!(spawn_pane(&mut server, &mut ordinary).is_err());
    assert_eq!(server.pane_ids.len(), before);
}

#[test]
fn unattached_client_path_overrides_and_spawn_environment_is_exported() {
    use std::os::fd::AsFd;
    let (mut server, mut context) = fixture();
    context.argv =
        vec![b"printf '%s|%s|%s|%s' \"$PATH\" \"$SHELL\" \"$RMUX_PANE\" \"$SPAWN_VALUE\"".to_vec()];
    let mut client = Environment::new();
    client.set(b"PATH", Default::default(), b"/custom/client/path");
    context.client_environment = Some(client);
    context
        .environment
        .set(b"SPAWN_VALUE", Default::default(), b"overridden");
    spawn_window(&mut server, &mut context).unwrap();
    let pane = context.pane.unwrap();
    let pid = server.panes.get(pane).unwrap().pid.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut data = Vec::new();
    let mut status = None;
    loop {
        let mut buffer = [0; 1024];
        let fd = server.panes.get(pane).unwrap().fd.as_ref().unwrap();
        let eof = match rmux_sys::fd::read(fd.as_fd(), &mut buffer) {
            Ok(0) => true,
            Ok(n) => {
                data.extend_from_slice(&buffer[..n]);
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(e) if e.raw_os_error() == Some(5) => true,
            Err(e) => panic!("{e}"),
        };
        if status.is_none() {
            status = rmux_sys::proc::wait_process(pid, true).unwrap();
        }
        if eof && status.is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            if status.is_none() {
                let _ = rmux_sys::proc::terminate_process(pid);
                let _ = rmux_sys::proc::wait_process(pid, false);
            }
            panic!("spawn environment child timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(rmux_sys::proc::exit_code(status.unwrap()), 0);
    let p = server.panes.get(pane).unwrap();
    let expected = [
        b"/custom/client/path|".as_slice(),
        &p.shell,
        format!("|%{}|overridden", p.public_id).as_bytes(),
    ]
    .concat();
    assert_eq!(data, expected);
}

#[test]
fn editor_process_is_modal_and_returns_the_written_file() {
    let (mut server, mut context) = fixture();
    context.flags = SpawnFlags::EMPTY;
    spawn_window(&mut server, &mut context).unwrap();
    let mut parser = StringOnlyParser;
    server.options.set_string(
        server.options.global,
        b"editor",
        false,
        b"printf edited >",
        &mut parser,
    );
    let calls = Rc::new(RefCell::new(Vec::new()));
    let capture = calls.clone();
    let editor = spawn_editor(
        &mut server,
        &context,
        b"initial",
        Box::new(move |_, _, data| capture.borrow_mut().push(data)),
    )
    .unwrap();
    let state = server.editors.get(editor).unwrap();
    let pane = state.pane;
    let pid = state.pid;
    let path = state.path.clone();
    let p = server.panes.get(pane).unwrap();
    let window = p.window;
    assert_eq!(server.windows.get(window).unwrap().modal, Some(pane));
    assert!(p.flags.contains(PaneFlags::FLOATOVERZOOM));
    assert_eq!(server.options.get_number(p.options, b"remain-on-exit"), 0);
    let status = rmux_sys::proc::wait_process(pid, false).unwrap().unwrap();
    let p = server.panes.get_mut(pane).unwrap();
    p.status = status;
    p.flags.insert(PaneFlags::STATUSREADY);
    spawn_editor_finish(&mut server, pane);
    assert_eq!(*calls.borrow(), vec![Some(b"edited".to_vec())]);
    assert!(!std::path::Path::new(std::str::from_utf8(&path).unwrap()).exists());
}
