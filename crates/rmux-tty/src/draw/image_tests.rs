// Ported from tmux tty.c @ 8f25579c
use super::*;
use rmux_emu::image::{ImageRegistry, SixelImage};
use std::num::NonZeroU32;

fn output(
    payload: &[u8],
    capability: bool,
    metrics: (u32, u32),
    viewport: (u32, u32, u32, u32),
) -> (Vec<u8>, Tty) {
    let mut state = TparmState::default();
    let mut tty = fixture(8, 4, &mut state);
    tty.xpixel = metrics.0;
    tty.ypixel = metrics.1;
    if capability {
        tty.term_mut().apply(b"Sxl", false, TtyTermFlags(0));
    }
    // Already full-width/full-height, so only the image cursor is written.
    tty.rupper = 0;
    tty.rlower = 3;
    let mut hyperlinks = HyperlinkRegistry::default();
    let mut images = ImageRegistry::default();
    let mut screen = Screen::new(8, 4, 0, ScreenResetPolicy::default(), &mut hyperlinks).unwrap();
    screen.bind_images(&mut images);
    let owner = screen.image_owner().unwrap();
    let data = SixelImage::parse(
        payload,
        2,
        NonZeroU32::new(1).unwrap(),
        NonZeroU32::new(1).unwrap(),
    )
    .unwrap();
    let id = images.store(owner, data, 0, 0);
    let image = images.get(owner, id).unwrap();
    let ctx = TtyCtx {
        s: &screen,
        cell: &DEFAULT_CELL,
        flags: TtyCtxFlags::WINDOW_BIGGER,
        data: TtyCommandData::SixelImage(image),
        ocx: image.px,
        ocy: image.py,
        orupper: 0,
        orlower: 3,
        xoff: 0,
        yoff: 0,
        rxoff: 0,
        ryoff: 0,
        sx: 8,
        sy: 4,
        bg: 8,
        defaults: DEFAULT_CELL,
        style_ctx: TtyStyleCtx::default(),
        wox: viewport.0,
        woy: viewport.1,
        wsx: viewport.2,
        wsy: viewport.3,
    };
    assert_eq!(tty.command(&mut state, TtyCommand::SixelImage, &ctx), None);
    (tty.out.iter().copied().collect(), tty)
}

#[test]
fn sixel_exact_wire_and_invalidated_state() {
    let (bytes, tty) = output(b"q\"1;1;2;2#0;2;100;0;0BB", true, (1, 1), (0, 0, 8, 4));
    assert_eq!(bytes, b"\x1b[1;1H\x1bP9;2q\"1;1;2;2#0;2;100;0;0#0BB\x1b\\");
    assert!(tty.flags.contains(TtyFlags::NOBLOCK));
    assert_eq!(
        (
            tty.cx, tty.cy, tty.rupper, tty.rlower, tty.rleft, tty.rright
        ),
        (u32::MAX, u32::MAX, u32::MAX, u32::MAX, u32::MAX, u32::MAX)
    );
}

#[test]
fn sixel_clips_left_top_and_uses_original_palette() {
    let (bytes, _) = output(b"q\"1;1;2;2#0;2;100;0;0BB", true, (1, 1), (1, 1, 1, 1));
    assert_eq!(bytes, b"\x1b[1;1H\x1bP9;2q\"1;1;1;1#0;2;100;0;0#0@\x1b\\");
}

#[test]
fn sixel_clips_right_bottom() {
    let (bytes, _) = output(b"q\"1;1;2;2#0;2;100;0;0BB", true, (1, 1), (0, 0, 1, 1));
    assert_eq!(bytes, b"\x1b[1;1H\x1bP9;2q\"1;1;1;1#0;2;100;0;0#0@\x1b\\");
}

#[test]
fn sixel_resamples_using_client_pixel_metrics() {
    let (bytes, _) = output(b"q\"1;1;1;1#0;2;100;0;0@", true, (2, 2), (0, 0, 8, 4));
    assert_eq!(bytes, b"\x1b[1;1H\x1bP9;2q\"1;1;2;2#0;2;100;0;0#0BB\x1b\\");
}

#[test]
fn fallback_is_not_cropped_to_the_visible_rectangle() {
    for (capability, metrics) in [(false, (1, 1)), (true, (0, 1)), (true, (1, 0))] {
        let (bytes, tty) = output(b"q\"1;1;2;2#0BB", capability, metrics, (1, 1, 1, 1));
        assert_eq!(bytes, b"\x1b[1;1HSIXEL IMAGE (2x2)\r\n++\r\n");
        assert!(tty.flags.contains(TtyFlags::NOBLOCK));
    }
}

#[test]
fn hidden_image_and_unprintable_sixel_leave_output_state_alone() {
    for (payload, viewport) in [
        (b"q\"1;1;2;2#0BB".as_slice(), (2, 0, 1, 1)),
        (b"q\"1;1;2;2".as_slice(), (0, 0, 8, 4)),
    ] {
        let (bytes, tty) = output(payload, true, (1, 1), viewport);
        assert!(bytes.is_empty());
        assert!(!tty.flags.contains(TtyFlags::NOBLOCK));
        assert_eq!((tty.rupper, tty.rlower), (0, 3));
    }
}

#[test]
fn sixel_clips_both_edges_without_changing_c_source_offsets() {
    let (bytes, _) = output(b"q\"1;1;4;4#0;2;100;0;0NNNN", true, (1, 1), (1, 1, 2, 2));
    assert_eq!(bytes, b"\x1b[1;1H\x1bP9;2q\"1;1;2;2#0;2;100;0;0#0BB\x1b\\");
}
