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

/// `TMUX_VERSION` as the binary reports it (`tmux.c:428-432`).
pub const VERSION: &str = rmux_emu::input::TMUX_VERSION;

/// `getversion` (`tmux.c:428-432`).
pub fn getversion() -> &'static str {
    VERSION
}

/// rmux's own release, independent of the tmux version it ports.
pub const RELEASE: &str = env!("CARGO_PKG_VERSION");
pub const GIT_SHA: &str = env!("RMUX_GIT_SHA");
pub const TARGET: &str = env!("RMUX_TARGET");

/// What `rmux version` prints.
pub fn release_line() -> String {
    format!("rmux {RELEASE} ({GIT_SHA}, {TARGET})")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_server_environment_constant() {
        assert_eq!(
            super::getversion().as_bytes(),
            rmux_server::options::environment::TMUX_VERSION
        );
        assert_eq!(super::VERSION, "next-3.9");
    }

    #[test]
    fn release_line_names_release_and_target() {
        let line = super::release_line();
        assert!(line.starts_with(&format!("rmux {} (", env!("CARGO_PKG_VERSION"))));
        assert!(line.ends_with(&format!(", {})", super::TARGET)));
    }
}
