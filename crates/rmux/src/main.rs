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

use std::{env, process::ExitCode};

const USAGE: &str = "usage: rmux [-2CDhlNuVv] [-c shell-command] [-f file] [-L socket-name]\n            [-S socket-path] [-T features] [command [flags]]";

#[derive(Debug, PartialEq)]
enum Action {
    Version,
    Help,
    Invalid,
    Run,
}

fn parse(args: &[String]) -> Action {
    let mut i = 0;
    let mut exclusive = false;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        let mut flags = arg[1..].char_indices().peekable();
        while let Some((_, flag)) = flags.next() {
            match flag {
                'V' => return Action::Version,
                'h' => return Action::Help,
                '2' | 'C' | 'l' | 'N' | 'q' | 'u' | 'v' => {}
                'D' => exclusive = true,
                'c' | 'f' | 'L' | 'S' | 'T' => {
                    exclusive |= flag == 'c';
                    if flags.peek().is_none() {
                        i += 1;
                        if i == args.len() {
                            return Action::Invalid;
                        }
                    }
                    break;
                }
                _ => return Action::Invalid,
            }
        }
        i += 1;
    }
    if exclusive && i < args.len() {
        Action::Invalid
    } else {
        Action::Run
    }
}

fn main() -> ExitCode {
    match parse(&env::args().skip(1).collect::<Vec<_>>()) {
        Action::Version => {
            println!(
                "rmux {} (tmux next-3.9 behavior)",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Action::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Action::Invalid => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        Action::Run => {
            eprintln!("rmux: server not implemented yet (P0)");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn top_level_flags() {
        for (args, action) in [
            (vec!["-V"], Action::Version),
            (vec!["-h"], Action::Help),
            (vec!["-Z"], Action::Invalid),
            (vec!["-S"], Action::Invalid),
            (vec!["-vv", "-Ltest", "new", "-d"], Action::Run),
            (vec!["-c", "echo x", "new"], Action::Invalid),
            (vec!["-D", "new"], Action::Invalid),
            (vec!["--", "-V"], Action::Run),
        ] {
            assert_eq!(
                parse(&args.into_iter().map(String::from).collect::<Vec<_>>()),
                action
            );
        }
    }
}
