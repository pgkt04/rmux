// Ported from tmux spawn.c @ 8f25579c
use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

fn options(argv: &[&[u8]]) -> LaunchOptions {
    LaunchOptions {
        shell: b"/bin/sh".to_vec(),
        argv: argv.iter().map(|v| v.to_vec()).collect(),
        environment: vec![b"PATH=/usr/bin:/bin".to_vec(), b"RMUX_PANE=%17".to_vec()],
        cwd: b"/".to_vec(),
        home: None,
        termios: None,
        backspace: 0x7f,
        size: Winsize {
            cols: 80,
            rows: 24,
            xpixel: 640,
            ypixel: 384,
        },
    }
}

fn capture(options: LaunchOptions, input: &[u8]) -> (Vec<u8>, i32) {
    let process = PreparedLaunch::new(options).unwrap().launch().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    let mut buffer = [0; 4096];
    let mut input_offset = 0;
    let mut status = None;
    loop {
        if input_offset < input.len() {
            match rmux_sys::fd::write(process.master.as_fd(), &input[input_offset..]) {
                Ok(n) => input_offset += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("pty write: {e}"),
            }
        }
        let eof = match rmux_sys::fd::read(process.master.as_fd(), &mut buffer) {
            Ok(0) => true,
            Ok(n) => {
                out.extend_from_slice(&buffer[..n]);
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(e) if e.raw_os_error() == Some(5) => true,
            Err(e) => panic!("pty read: {e}"),
        };
        if status.is_none() {
            status = rmux_sys::proc::wait_process(process.pid, true).unwrap();
        }
        if eof && status.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            if status.is_none() {
                rmux_sys::proc::terminate_process(process.pid).unwrap();
                rmux_sys::proc::wait_process(process.pid, false).unwrap();
            }
            panic!("child launch timed out");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    (out, rmux_sys::proc::exit_code(status.unwrap()))
}

#[test]
fn direct_arguments_and_shell_command_are_distinct() {
    let (out, code) = capture(options(&[b"printf", b"<%s>\\n", b"literal $SHELL ;"]), b"");
    assert_eq!(code, 0);
    assert_eq!(out, b"<literal $SHELL ;>\r\n");
    let (out, code) = capture(options(&[b"printf '%s|%s\\n' \"$0\" \"$RMUX_PANE\""]), b"");
    assert_eq!(code, 0);
    assert_eq!(out, b"sh|%17\r\n");
}

#[test]
fn login_shell_and_failed_exec_exit_status() {
    let (out, code) = capture(options(&[]), b"printf 'ARGV0=%s\\n' \"$0\"; exit 7\n");
    assert_eq!(code, 7);
    assert!(out.windows(9).any(|s| s == b"ARGV0=-sh"));
    let (_, code) = capture(options(&[b"rmux-no-such-program-92843", b"x"]), b"");
    assert_eq!(code, 1);
}

#[test]
fn cwd_falls_back_and_parent_directory_is_unchanged() {
    let before = std::env::current_dir().unwrap();
    let mut policy = options(&[b"printf '%s|%s\\n' \"$PWD\" \"$(pwd)\""]);
    policy.cwd = b"/no-such-rmux-directory-92381".to_vec();
    policy.home = Some(b"/tmp".to_vec());
    let (out, code) = capture(policy, b"");
    assert_eq!(code, 0);
    assert_eq!(out, b"/tmp|/tmp\r\n");
    assert_eq!(std::env::current_dir().unwrap(), before);
}

#[test]
fn terminal_controls_size_and_erase_policy() {
    let (_master, slave, _) = rmux_sys::pty::openpty().unwrap();
    let termios = rmux_sys::TermiosState::get(slave.as_fd()).unwrap();
    let mut policy = options(&[b"stty size; stty -a"]);
    policy.termios = Some(termios);
    policy.backspace = 8;
    let (out, code) = capture(policy, b"");
    assert_eq!(code, 0);
    assert!(out.starts_with(b"24 80\r\n"));
    assert!(
        out.windows(10).any(|s| s == b"erase = ^H") || out.windows(8).any(|s| s == b"erase ^H")
    );
}

#[test]
fn prepared_launch_rejects_nul_before_fork() {
    let mut policy = options(&[b"echo x"]);
    policy.cwd = b"/tmp\0other".to_vec();
    assert!(PreparedLaunch::new(policy).is_err());
}

#[test]
fn path_search_executes_plain_executable_file_through_sh() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("rmux-launch-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("plain-executable");
    std::fs::write(&file, b"printf 'fallback:%s\\n' \"$1\"\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut policy = options(&[b"plain-executable", b"argument"]);
    policy.environment = vec![[b"PATH=".as_slice(), dir.as_os_str().as_encoded_bytes()].concat()];
    let (out, code) = capture(policy, b"");
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(code, 0);
    assert_eq!(out, b"fallback:argument\r\n");
}
