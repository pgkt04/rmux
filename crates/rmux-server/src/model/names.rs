// Ported from tmux names.c @ 8f25579c
use super::state::{ModelError, Server, clean_name};
use crate::ids::{PaneId, WindowId};
use rmux_util::bytes::cstr;

pub const NAME_INTERVAL: i64 = 500_000;

pub trait NamesRuntime {
    fn automatic_rename(&self, server: &Server, window: WindowId) -> bool;
    fn expand_name(&mut self, server: &mut Server, window: WindowId, pane: PaneId) -> Vec<u8>;
    fn name_timer(&mut self, window: WindowId, delay_usec: Option<u64>);
    fn redraw_name(&mut self, window: WindowId);
}

pub fn name_time_remaining(previous: (i64, i64), now: (i64, i64)) -> u64 {
    let mut sec = now.0 - previous.0;
    let mut usec = now.1 - previous.1;
    if usec < 0 {
        sec -= 1;
        usec += 1_000_000;
    }
    if sec != 0 || usec > NAME_INTERVAL {
        return 0;
    }
    (NAME_INTERVAL - usec) as u64
}

pub fn parse_window_name(input: &[u8]) -> Vec<u8> {
    let mut name = cstr(input);
    if name.first() == Some(&b'"') {
        name = &name[1..];
    }
    name = &name[..name.iter().position(|b| *b == b'"').unwrap_or(name.len())];
    if name.starts_with(b"exec ") {
        name = &name[5..];
    }
    while matches!(name.first(), Some(b' ' | b'-')) {
        name = &name[1..];
    }
    name = &name[..name.iter().position(|b| *b == b' ').unwrap_or(name.len())];
    while name.len() > 1
        && !rmux_sys::locale::is_alnum(name[name.len() - 1])
        && !name[name.len() - 1].is_ascii_punctuation()
    {
        name = &name[..name.len() - 1];
    }
    if name.first() == Some(&b'/') {
        return clean_name(&rmux_sys::path::basename(name), false).unwrap_or_default();
    }
    clean_name(name, false).unwrap_or_default()
}

pub fn default_window_name(server: &Server, window: WindowId) -> Vec<u8> {
    let Some(pane) = server
        .windows
        .get(window)
        .and_then(|w| w.active)
        .and_then(|p| server.panes.get(p))
    else {
        return Vec::new();
    };
    if pane.argv.is_empty() {
        return parse_window_name(&pane.shell);
    }
    let mut command = Vec::new();
    for (index, arg) in pane.argv.iter().enumerate() {
        if index != 0 {
            command.push(b' ');
        }
        command.extend_from_slice(&crate::cmd::arguments::escape(arg));
    }
    if command.is_empty() {
        parse_window_name(&pane.shell)
    } else {
        parse_window_name(&command)
    }
}

pub fn check_window_name(
    server: &mut Server,
    window: WindowId,
    runtime: &mut impl NamesRuntime,
) -> Result<(), ModelError> {
    let Some(w) = server.windows.get(window) else {
        return Err(ModelError::StaleId);
    };
    let Some(pane) = w.active else {
        return Ok(());
    };
    if !runtime.automatic_rename(server, window)
        || !server
            .panes
            .get(pane)
            .is_some_and(|p| p.flags.contains(super::PaneFlags::CHANGED))
    {
        return Ok(());
    }
    let left = name_time_remaining(w.name_time, server.current_time);
    if left != 0 {
        let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
        if !w.name_timer_pending {
            w.name_timer_pending = true;
            runtime.name_timer(window, Some(left));
        }
        return Ok(());
    }
    let w = server.windows.get_mut(window).ok_or(ModelError::StaleId)?;
    w.name_time = server.current_time;
    w.name_timer_pending = false;
    runtime.name_timer(window, None);
    if let Some(p) = server.panes.get_mut(pane) {
        p.flags.remove(super::PaneFlags::CHANGED);
    }
    let name = runtime.expand_name(server, window, pane);
    if server.windows.get(window).is_some_and(|w| w.name != name) {
        super::window::window_set_name(server, window, &name, true)?;
        runtime.redraw_name(window);
    }
    Ok(())
}

pub fn name_timer_fired(server: &mut Server, window: WindowId) {
    if let Some(w) = server.windows.get_mut(window) {
        w.name_timer_pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Runtime {
        timers: Vec<Option<u64>>,
        redraws: usize,
    }
    impl NamesRuntime for Runtime {
        fn automatic_rename(&self, _: &Server, _: WindowId) -> bool {
            true
        }
        fn expand_name(&mut self, _: &mut Server, _: WindowId, _: PaneId) -> Vec<u8> {
            b"#(unsafe)".to_vec()
        }
        fn name_timer(&mut self, _: WindowId, delay: Option<u64>) {
            self.timers.push(delay);
        }
        fn redraw_name(&mut self, _: WindowId) {
            self.redraws += 1;
        }
    }
    #[test]
    fn one_pending_timer_and_accepted_check_clears_changed() {
        let mut server = Server::new();
        let window = super::super::window::window_create(&mut server, 80, 24, 0, 0).unwrap();
        let pane = super::super::pane::pane_create(&mut server, window, 80, 24, 0).unwrap();
        server.windows.get_mut(window).unwrap().active = Some(pane);
        server
            .panes
            .get_mut(pane)
            .unwrap()
            .flags
            .insert(super::super::PaneFlags::CHANGED);
        server.windows.get_mut(window).unwrap().name_time = (10, 0);
        server.current_time = (10, 100_000);
        let mut runtime = Runtime::default();
        check_window_name(&mut server, window, &mut runtime).unwrap();
        check_window_name(&mut server, window, &mut runtime).unwrap();
        assert_eq!(runtime.timers, [Some(400_000)]);
        assert!(
            server
                .panes
                .get(pane)
                .unwrap()
                .flags
                .contains(super::super::PaneFlags::CHANGED)
        );
        name_timer_fired(&mut server, window);
        server.current_time = (10, 500_000);
        check_window_name(&mut server, window, &mut runtime).unwrap();
        assert_eq!(runtime.timers, [Some(400_000), None]);
        assert!(
            !server
                .panes
                .get(pane)
                .unwrap()
                .flags
                .contains(super::super::PaneFlags::CHANGED)
        );
        assert_eq!(server.windows.get(window).unwrap().name, b"_(unsafe)");
        assert_eq!(runtime.redraws, 1);
        server.current_time = (11, 0);
        check_window_name(&mut server, window, &mut runtime).unwrap();
        assert_eq!(runtime.redraws, 1);
        server.panes.get_mut(pane).unwrap().argv = vec![b"exec /usr/bin/vi file".to_vec()];
        assert_eq!(default_window_name(&server, window), b"vi");
        server.panes.get_mut(pane).unwrap().argv.clear();
        server.panes.get_mut(pane).unwrap().shell = b"/bin/sh".to_vec();
        assert_eq!(default_window_name(&server, window), b"sh");
        server.windows.get_mut(window).unwrap().active = None;
        assert!(default_window_name(&server, window).is_empty());
    }
    #[test]
    fn parser_matches_pinned_c() {
        use std::process::Command;
        let source = Command::new("git")
            .args(["-C", "/Users/j/fun/tmux", "show", "8f25579c:names.c"])
            .output();
        let Ok(source) = source else {
            eprintln!("skipping names C reference: pinned source unavailable");
            return;
        };
        if !source.status.success() {
            eprintln!("skipping names C reference: pinned source unavailable");
            return;
        }
        let source = String::from_utf8(source.stdout).unwrap();
        let start = source.find("char *\nparse_window_name(").unwrap();
        let harness = format!(
            "#include <sys/types.h>\n#include <ctype.h>\n#include <libgen.h>\n#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n#define xstrdup strdup\nstatic char *clean_name(const char *s,int untrusted) {{ (void)untrusted; return strdup(s); }}\n{}\nint main(int argc,char **argv) {{ for(int i=1;i<argc;i++) {{ char *s=parse_window_name(argv[i]); puts(s); free(s); }} }}\n",
            &source[start..]
        );
        let dir = std::env::temp_dir().join(format!("rmux-names-cref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.c"), harness).unwrap();
        let build = Command::new("cc")
            .arg(dir.join("probe.c"))
            .arg("-o")
            .arg(dir.join("probe"))
            .output()
            .expect("C compiler required with pinned source");
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let fixtures = [
            "",
            "\"exec /usr/bin/vi file\"",
            "exec -- /bin/sh",
            " -bash -l",
            "/usr/bin/",
            "/",
            "printf;",
            "x\t\n",
            "a\"b",
            "exec  --x",
        ];
        let output = Command::new(dir.join("probe"))
            .args(fixtures)
            .output()
            .unwrap();
        assert!(output.status.success());
        for (fixture, expected) in fixtures.iter().zip(output.stdout.split(|b| *b == b'\n')) {
            assert_eq!(
                parse_window_name(fixture.as_bytes()),
                expected,
                "{fixture:?}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn parsing_is_not_shell_parsing() {
        for (input, expected) in [
            (b"\"exec /usr/bin/vi file\"".as_slice(), b"vi".as_slice()),
            (b"exec -- /bin/sh", b"sh"),
            (b" -bash -l", b"bash"),
            (b"/usr/bin/", b"bin"),
            (b"printf;", b"printf;"),
            (b"x\t\n", b"x"),
            (b"\t", b"\\t"),
            (b"\xff", b""),
        ] {
            assert_eq!(parse_window_name(input), expected);
        }
    }
    #[test]
    fn wall_clock_throttle_boundaries() {
        assert_eq!(name_time_remaining((10, 100), (10, 100)), 500_000);
        assert_eq!(name_time_remaining((10, 100), (10, 500_099)), 1);
        assert_eq!(name_time_remaining((10, 100), (10, 500_100)), 0);
        assert_eq!(name_time_remaining((10, 100), (10, 500_101)), 0);
        assert_eq!(name_time_remaining((10, 900_000), (11, 100_000)), 300_000);
        assert_eq!(name_time_remaining((10, 100), (9, 100)), 0);
    }
}
