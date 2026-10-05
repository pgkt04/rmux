// Ported from tmux input.c @ 8f25579c
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::input::{InputCtx, InputEffect, InputPolicy, InputSink};
use rmux_emu::screen::write::{ScreenOnlySink, ScreenWriteCtx, ScreenWritePolicy};
use rmux_emu::screen::{Screen, ScreenResetPolicy};

#[derive(Default)]
struct Replies(Vec<Vec<u8>>);
impl InputSink for Replies {
    fn effect(&mut self, effect: InputEffect<'_>) {
        if let InputEffect::Reply(bytes) = effect {
            self.0.push(bytes.to_vec());
        }
    }
    fn reply(&mut self, bytes: &[u8]) {
        self.0.push(bytes.to_vec());
    }
}

#[test]
fn graphics_replies_follow_compile_feature_not_runtime_policy() {
    for sixel in [false, true] {
        let mut links = HyperlinkRegistry::default();
        let mut screen = Screen::new(8, 5, 0, ScreenResetPolicy::default(), &mut links).unwrap();
        let mut tty = ScreenOnlySink;
        let mut writer = ScreenWriteCtx::start(
            &mut screen,
            &mut tty,
            ScreenWritePolicy::default(),
            &mut links,
            #[cfg(feature = "sixel")]
            None,
        );
        let mut input = InputCtx::new();
        let mut replies = Replies::default();
        let policy = InputPolicy {
            sixel,
            has_pane: false,
            ..InputPolicy::default()
        };
        input.parse(
            &mut writer,
            None,
            &policy,
            &mut replies,
            b"\x1b[c\x1b[?1;1S\x1b[?1;2S\x1b[?1;4S\x1b[?2;1;9S\x1b[?1;1;0;0S",
        );
        writer.finish();
        let expected: Vec<Vec<u8>> = if cfg!(feature = "sixel") {
            [
                &b"\x1b[?1;2;4c"[..],
                &b"\x1b[?1;0;1024S"[..],
                &b"\x1b[?1;0;1024S"[..],
                &b"\x1b[?1;0;1024S"[..],
                &b"\x1b[?2;3;9S"[..],
            ]
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect()
        } else {
            vec![b"\x1b[?1;2c".to_vec()]
        };
        assert_eq!(replies.0, expected);
        #[cfg(feature = "sixel")]
        assert!(screen.image_owner().is_none());
    }
}
