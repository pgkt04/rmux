// Ported from tmux tmux.c @ 8f25579c
/*
Copyright (c) Various Authors
Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
Copyright (c) 2026 Jacky and rmux contributors

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
*/

#![forbid(unsafe_code)]

mod client;
mod paths;
mod version;

use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use rmux_server::client::ClientFlags;
use rmux_server::options::environment::EnvironmentFlags;
use rmux_server::server::{Startup, server_start};
use rmux_sys::client as sys;
use rmux_tty::features::{TtyFeatures, parse_features_bytes};
use rmux_util::bytes::find_case_insensitive;

/// Program name used in usage, errors and `-V` (`getprogname()`).
pub const PROGNAME: &str = "rmux";

/// `usage` text (`tmux.c:73-76`) with the rmux program name.
pub const USAGE: &str = "usage: rmux [-2CDhlNuVv] [-c shell-command] [-f file] [-L socket-name]\n            [-S socket-path] [-T features] [command [flags]]\n       rmux omp-plugin [directory]    export the bundled omp plugin\n";

/// Internal reexec marker for the server child (replaces the `fork` in
/// `server_start`, `server.c:189-193`).
pub const INTERNAL_SERVER_FLAG: &str = "--rmux-internal-server";

/// Parsed command line (`tmux.c:434-536`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MainConfig {
    pub flags: ClientFlags,
    pub feat: TtyFeatures,
    pub socket_path: Vec<u8>,
    pub label: Option<Vec<u8>>,
    pub shell_command: Option<Vec<u8>>,
    pub cfg_files: Vec<Vec<u8>>,
    pub cfg_quiet: bool,
    pub command: Vec<Vec<u8>>,
    pub log_level: u32,
}

/// An early exit from option parsing: `text` goes to stdout when `code` is
/// zero, else to stderr (`tmux.c:73`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub code: i32,
    pub text: String,
}

fn usage(code: i32) -> Exit {
    Exit {
        code,
        text: USAGE.to_string(),
    }
}

/// getopt diagnostics as tmux's bundled BSD getopt prints them on every
/// platform (`compat/getopt_long.c:153-158`).
fn bad_option(flag: u8) -> Exit {
    Exit {
        code: 1,
        text: format!("{PROGNAME}: unknown option -- {}\n{USAGE}", flag as char),
    }
}

fn missing_argument(flag: u8) -> Exit {
    Exit {
        code: 1,
        text: format!(
            "{PROGNAME}: option requires an argument -- {}\n{USAGE}",
            flag as char
        ),
    }
}

const OPTSTRING: &[u8] = b"2c:CDdf:hlL:NqS:T:uUvV";

/// POSIX `getopt` over `2c:CDdf:hlL:NqS:T:uUvV` (`tmux.c:467-536`). `args`
/// excludes `argv[0]`; `login` is the `argv[0]` `-` test (`tmux.c:457-458`);
/// `default_cfg` is the expanded `TMUX_CONF` list (`tmux.c:465`).
pub fn parse_args(
    args: &[Vec<u8>],
    login: bool,
    default_cfg: Vec<Vec<u8>>,
) -> Result<MainConfig, Exit> {
    let mut config = MainConfig {
        cfg_files: default_cfg,
        cfg_quiet: true,
        ..MainConfig::default()
    };
    if login {
        config.flags.insert(ClientFlags::LOGIN);
    }
    let mut fflag = false;
    let mut path: Option<Vec<u8>> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == b"--" {
            i += 1;
            break;
        }
        if arg.len() < 2 || arg[0] != b'-' {
            break;
        }
        i += 1;
        let mut j = 1;
        while j < arg.len() {
            let flag = arg[j];
            j += 1;
            let Some(pos) = OPTSTRING.iter().position(|&c| c == flag && c != b':') else {
                return Err(bad_option(flag));
            };
            let takes_arg = OPTSTRING.get(pos + 1) == Some(&b':');
            let optarg: Option<&[u8]> = if takes_arg {
                if j < arg.len() {
                    let value = &arg[j..];
                    j = arg.len();
                    Some(value)
                } else if i < args.len() {
                    i += 1;
                    Some(&args[i - 1])
                } else {
                    return Err(missing_argument(flag));
                }
            } else {
                None
            };
            match flag {
                b'2' => parse_features_bytes(b"256", b":,", &mut config.feat),
                b'c' => config.shell_command = optarg.map(<[u8]>::to_vec),
                b'D' => config.flags.insert(ClientFlags::NOFORK),
                b'C' => {
                    if config.flags.contains(ClientFlags::CONTROL) {
                        config.flags.insert(ClientFlags::CONTROLCONTROL);
                    } else {
                        config.flags.insert(ClientFlags::CONTROL);
                    }
                }
                b'f' => {
                    if !fflag {
                        fflag = true;
                        config.cfg_files.clear();
                    }
                    config.cfg_files.push(optarg.unwrap_or_default().to_vec());
                    config.cfg_quiet = false;
                }
                b'h' => return Err(usage(0)),
                b'V' => {
                    return Err(Exit {
                        code: 0,
                        text: format!("{PROGNAME} {}\n", version::getversion()),
                    });
                }
                b'l' => config.flags.insert(ClientFlags::LOGIN),
                b'L' => config.label = optarg.map(<[u8]>::to_vec),
                b'N' => config.flags.insert(ClientFlags::NOSTARTSERVER),
                b'q' => {}
                b'S' => path = optarg.map(<[u8]>::to_vec),
                b'T' => parse_features_bytes(optarg.unwrap_or_default(), b":,", &mut config.feat),
                b'u' => config.flags.insert(ClientFlags::UTF8),
                b'v' => config.log_level += 1,
                // `-d` and `-U` have no case label (`tmux.c:526-527`).
                _ => return Err(usage(1)),
            }
        }
    }
    config.command = args[i..].to_vec();
    if config.shell_command.is_some() && !config.command.is_empty() {
        return Err(usage(1));
    }
    if config.flags.contains(ClientFlags::NOFORK) && !config.command.is_empty() {
        return Err(usage(1));
    }
    config.socket_path = path.unwrap_or_default();
    Ok(config)
}

/// UTF-8 detection (`tmux.c:544-564`) reading `RMUX` instead of `TMUX`.
pub fn detect_utf8(
    rmux: Option<&[u8]>,
    lc_all: Option<&[u8]>,
    lc_ctype: Option<&[u8]>,
    lang: Option<&[u8]>,
) -> bool {
    if rmux.is_some() {
        return true;
    }
    let s = [lc_all, lc_ctype, lang]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or(b"");
    find_case_insensitive(s, b"UTF-8").is_some() || find_case_insensitive(s, b"UTF8").is_some()
}

/// Socket path precedence (`tmux.c:598-619`): `-S`, then `-L`, then the first
/// field of `$RMUX` before the comma, then `make_label`.
pub fn socket_path_from_env(rmux: Option<&[u8]>) -> Option<Vec<u8>> {
    let s = rmux?;
    if s.is_empty() || s[0] == b',' {
        return None;
    }
    let end = s.iter().position(|&c| c == b',').unwrap_or(s.len());
    Some(s[..end].to_vec())
}

fn exit_with(exit: Exit) -> ! {
    if exit.code == 0 {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(exit.text.as_bytes());
        let _ = out.flush();
    } else {
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(exit.text.as_bytes());
        let _ = err.flush();
    }
    std::process::exit(exit.code);
}

fn errx(text: &[u8]) -> ! {
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(PROGNAME.as_bytes());
    let _ = err.write_all(b": ");
    let _ = err.write_all(text);
    let _ = err.write_all(b"\n");
    let _ = err.flush();
    std::process::exit(1);
}

/// The reexecuted server child (`server_start` child branch, `server.c:189-264`).
/// argv: `--rmux-internal-server <peer_fd> <lock_fd|-1> <socket_path> <flags> <log_level> [cfg files...]`.
fn internal_server(args: &[Vec<u8>]) -> ! {
    rmux_util::log::init_crash_reporting("server");
    // tmux.c:445-455 run before the fork, so tmux's server inherits LC_CTYPE
    // and LC_TIME; a re-exec'd server starts in the C locale and must set
    // them again (libc wcwidth/mbtowc depend on it off macOS).
    if let Err(e) = rmux_sys::locale::setup_ctype() {
        errx(&e.message());
    }
    rmux_sys::locale::setup_time();
    let parse_fd = |s: &[u8]| -> i32 {
        std::str::from_utf8(s)
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or_else(|| errx(b"bad internal server descriptor"))
    };
    if args.len() < 5 {
        errx(b"bad internal server arguments");
    }
    let peer_fd = parse_fd(&args[0]);
    let lock_fd = parse_fd(&args[1]);
    let socket_path = args[2].clone();
    let flags = std::str::from_utf8(&args[3])
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| errx(b"bad internal server flags"));
    let log_level = std::str::from_utf8(&args[4])
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| errx(b"bad internal server log level"));
    if peer_fd < 0 {
        errx(b"bad internal server descriptor");
    }
    for _ in 0..log_level {
        rmux_util::log::add_level();
    }
    let initial_peer = Some(sys::take_inherited_fd(peer_fd));
    let lock = (lock_fd >= 0).then(|| sys::take_inherited_fd(lock_fd));
    if let Err(e) = rmux_sys::server::detach_session() {
        rmux_util::fatal!("setsid failed: {e}");
    }
    let code = match server_start(Startup {
        socket_path,
        flags: ClientFlags::from_bits_retain(flags),
        initial_peer,
        lock,
        activation_listener: None,
        config_files: args[5..].to_vec(),
    }) {
        Ok(code) => code,
        Err(e) => rmux_util::fatal!("server failed: {e}"),
    };
    std::process::exit(code);
}

fn main() {
    #[cfg(all(feature = "systemd", target_os = "linux"))]
    let activation_listener = match rmux_sys::systemd::take_activation_listener() {
        Ok(listener) => listener,
        Err(error) => {
            let cause = error
                .raw_os_error()
                .map(rmux_sys::strerror)
                .unwrap_or_else(|| error.to_string().into_bytes());
            eprintln!("systemd socket error ({})", String::from_utf8_lossy(&cause));
            std::process::exit(1);
        }
    };
    #[cfg(not(all(feature = "systemd", target_os = "linux")))]
    let activation_listener = None;
    let raw_args: Vec<Vec<u8>> = std::env::args_os().map(OsStringExt::into_vec).collect();
    let argv0 = raw_args.first().cloned().unwrap_or_default();
    let args = &raw_args[raw_args.len().min(1)..];

    if args
        .first()
        .is_some_and(|a| a == INTERNAL_SERVER_FLAG.as_bytes())
    {
        internal_server(&args[1..]);
    }

    if args.first().is_some_and(|arg| arg == b"omp-plugin") {
        if args.len() > 2 {
            errx(b"usage: rmux omp-plugin [directory]");
        }
        if let Some(directory) = args.get(1) {
            let directory = std::path::Path::new(std::ffi::OsStr::from_bytes(directory));
            if let Err(error) = std::fs::create_dir_all(directory) {
                errx(error.to_string().as_bytes());
            }
            for (name, contents) in [
                (
                    "package.json",
                    include_bytes!("../omp/package.json").as_slice(),
                ),
                ("rmux.ts", include_bytes!("../omp/rmux.ts").as_slice()),
            ] {
                if let Err(error) = std::fs::write(directory.join(name), contents) {
                    errx(error.to_string().as_bytes());
                }
            }
            println!("{}", directory.display());
            return;
        }
        if let Err(error) = std::io::stdout()
            .lock()
            .write_all(include_bytes!("../omp/rmux.ts"))
        {
            errx(error.to_string().as_bytes());
        }
        return;
    }

    // tmux.c:445-455
    if let Err(e) = rmux_sys::locale::setup_ctype() {
        errx(&e.message());
    }
    rmux_sys::locale::setup_time();

    let login = argv0.first() == Some(&b'-');

    // tmux.c:460-465: the globals live in a model Server for the client's lifetime.
    let mut server = rmux_server::model::Server::new();
    for var in sys::environ() {
        server
            .global_environment
            .put(&var, EnvironmentFlags::default());
    }
    if let Some(cwd) = paths::find_cwd() {
        server
            .global_environment
            .set(b"PWD", EnvironmentFlags::default(), &cwd);
    }
    let default_cfg = paths::expand_paths(
        paths::RMUX_CONF,
        true,
        paths::find_home(),
        &paths::env_lookup,
    );

    let mut config = match parse_args(args, login, default_cfg) {
        Ok(config) => config,
        Err(exit) => exit_with(exit),
    };
    rmux_util::log::init_crash_reporting(if config.flags.contains(ClientFlags::NOFORK) {
        "server"
    } else {
        "client"
    });
    for _ in 0..config.log_level {
        rmux_util::log::add_level();
    }

    // tmux.c:551-564
    if detect_utf8(
        sys::getenv("RMUX").as_deref(),
        sys::getenv("LC_ALL").as_deref(),
        sys::getenv("LC_CTYPE").as_deref(),
        sys::getenv("LANG").as_deref(),
    ) {
        config.flags.insert(ClientFlags::UTF8);
    }

    // tmux.c:566-596: the same defaults the reexecuted server applies.
    rmux_server::server::run::initialize_process_options(&mut server);

    // tmux.c:598-620
    if config.socket_path.is_empty() && config.label.is_none() {
        if let Some(path) = socket_path_from_env(sys::getenv("RMUX").as_deref()) {
            config.socket_path = path;
        }
    }
    if config.socket_path.is_empty() {
        match paths::make_label(config.label.as_deref(), rmux_sys::proc::getuid().0) {
            Ok(path) => config.socket_path = path,
            Err(cause) => {
                let mut err = std::io::stderr().lock();
                let _ = err.write_all(&cause);
                let _ = err.write_all(b"\n");
                let _ = err.flush();
                std::process::exit(1);
            }
        }
        config.flags.insert(ClientFlags::DEFAULTSOCKET);
    }

    // tmux.c:624
    std::process::exit(client::run(config, server, activation_listener));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    fn parse(list: &[&str]) -> Result<MainConfig, Exit> {
        parse_args(&args(list), false, vec![b"/etc/tmux.conf".to_vec()])
    }

    #[test]
    fn version_and_help() {
        assert_eq!(
            parse(&["-V"]).unwrap_err(),
            Exit {
                code: 0,
                text: format!("rmux {}\n", version::VERSION)
            }
        );
        assert_eq!(
            parse(&["-h"]).unwrap_err(),
            Exit {
                code: 0,
                text: USAGE.to_string()
            }
        );
        assert_eq!(parse(&["-x"]).unwrap_err().code, 1);
        assert!(parse(&["-x"]).unwrap_err().text.ends_with(USAGE));
        assert!(parse(&["-x"]).unwrap_err().text.contains("option -- "));
        assert_eq!(parse(&["-S"]).unwrap_err().code, 1);
        assert!(
            parse(&["-S"])
                .unwrap_err()
                .text
                .contains("requires an argument")
        );
        // -d and -U are in the string but hit default: usage(1).
        assert_eq!(parse(&["-d"]).unwrap_err(), usage(1));
        assert_eq!(parse(&["-U"]).unwrap_err(), usage(1));
    }

    #[test]
    fn every_option_has_its_effect() {
        let c = parse(&[
            "-2",
            "-C",
            "-l",
            "-N",
            "-q",
            "-u",
            "-vv",
            "-Lmain",
            "-S",
            "/s",
            "-Tfoo:RGB",
            "-c",
            "echo",
        ])
        .unwrap();
        assert!(c.flags.contains(ClientFlags::CONTROL));
        assert!(!c.flags.contains(ClientFlags::CONTROLCONTROL));
        assert!(
            c.flags
                .contains(ClientFlags::LOGIN | ClientFlags::NOSTARTSERVER | ClientFlags::UTF8)
        );
        assert_eq!(c.log_level, 2);
        assert_eq!(c.label.as_deref(), Some(&b"main"[..]));
        assert_eq!(c.socket_path, b"/s");
        assert_eq!(c.shell_command.as_deref(), Some(&b"echo"[..]));
        let mut feat = TtyFeatures::default();
        parse_features_bytes(b"256", b":,", &mut feat);
        parse_features_bytes(b"foo:RGB", b":,", &mut feat);
        assert_eq!(c.feat, feat);
        assert!(c.command.is_empty());
        assert!(c.cfg_quiet);

        let c = parse(&["-CC", "-D"]).unwrap();
        assert!(
            c.flags
                .contains(ClientFlags::CONTROLCONTROL | ClientFlags::NOFORK)
        );

        let c = parse(&["-f", "a", "-fb", "new", "-d"]).unwrap();
        assert_eq!(c.cfg_files, vec![b"a".to_vec(), b"b".to_vec()]);
        assert!(!c.cfg_quiet);
        assert_eq!(c.command, args(&["new", "-d"]));

        let c = parse(&["ls"]).unwrap();
        assert_eq!(c.cfg_files, vec![b"/etc/tmux.conf".to_vec()]);
        assert_eq!(c.command, args(&["ls"]));
    }

    #[test]
    fn getopt_stops_at_first_non_option_and_double_dash() {
        let c = parse(&["--", "-V"]).unwrap();
        assert_eq!(c.command, args(&["-V"]));
        let c = parse(&["new", "-V"]).unwrap();
        assert_eq!(c.command, args(&["new", "-V"]));
        let c = parse(&["-", "x"]).unwrap();
        assert_eq!(c.command, args(&["-", "x"]));
        let c = parse(&["-L", "a", "-L", "b"]).unwrap();
        assert_eq!(c.label.as_deref(), Some(&b"b"[..]));
    }

    #[test]
    fn exclusive_options_reject_commands() {
        assert_eq!(parse(&["-c", "echo", "new"]).unwrap_err(), usage(1));
        assert_eq!(parse(&["-D", "new"]).unwrap_err(), usage(1));
        let c = parse_args(&args(&["ls"]), true, Vec::new()).unwrap();
        assert!(c.flags.contains(ClientFlags::LOGIN));
    }

    #[test]
    fn utf8_detection_and_socket_env() {
        assert!(detect_utf8(Some(b"/x,1,2"), None, None, None));
        assert!(detect_utf8(None, Some(b"en_US.UTF-8"), None, None));
        assert!(detect_utf8(None, Some(b""), Some(b"C.utf8"), None));
        assert!(detect_utf8(None, None, None, Some(b"en_GB.utf-8")));
        assert!(!detect_utf8(None, Some(b"C"), Some(b"en_US.UTF-8"), None));
        assert!(!detect_utf8(None, None, None, None));
        assert_eq!(
            socket_path_from_env(Some(b"/tmp/s,123,0")).unwrap(),
            b"/tmp/s"
        );
        assert_eq!(socket_path_from_env(Some(b"/tmp/s")).unwrap(), b"/tmp/s");
        assert_eq!(socket_path_from_env(Some(b",123")), None);
        assert_eq!(socket_path_from_env(Some(b"")), None);
        assert_eq!(socket_path_from_env(None), None);
    }
}
