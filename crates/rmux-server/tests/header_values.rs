// Ported from tmux tmux.h @ 8f25579c
use std::{
    fs,
    process::{Command, Stdio},
};
#[test]
fn pinned_header_values() {
    let source = std::env::var_os("RMUX_TMUX_SOURCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/Users/j/fun/tmux"));
    if !source.join(".git").exists() {
        eprintln!("SKIP pinned header comparison: set RMUX_TMUX_SOURCE to the tmux git checkout");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rmux-g00-values-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let mut archive = Command::new("git")
        .arg("-C")
        .arg(&source)
        .args(["archive", "8f25579c"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let status = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&dir)
        .stdin(archive.stdout.take().unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(archive.wait().unwrap().success());
    let values: &[(&str, i64)] = &[
        (
            "KEYC_TYPE_UNICODE",
            rmux_util::key::KeyCodeType::Unicode as i64,
        ),
        ("KEYC_TYPE_USER", rmux_util::key::KeyCodeType::User as i64),
        (
            "KEYC_TYPE_FUNCTION",
            rmux_util::key::KeyCodeType::Function as i64,
        ),
        (
            "KEYC_TYPE_MOUSEMOVE",
            rmux_util::key::KeyCodeType::Mousemove as i64,
        ),
        (
            "KEYC_TYPE_MOUSEDOWN",
            rmux_util::key::KeyCodeType::Mousedown as i64,
        ),
        (
            "KEYC_TYPE_MOUSEUP",
            rmux_util::key::KeyCodeType::Mouseup as i64,
        ),
        (
            "KEYC_TYPE_MOUSEDRAG",
            rmux_util::key::KeyCodeType::Mousedrag as i64,
        ),
        (
            "KEYC_TYPE_MOUSEDRAGEND",
            rmux_util::key::KeyCodeType::Mousedragend as i64,
        ),
        (
            "KEYC_TYPE_WHEELDOWN",
            rmux_util::key::KeyCodeType::Wheeldown as i64,
        ),
        (
            "KEYC_TYPE_WHEELUP",
            rmux_util::key::KeyCodeType::Wheelup as i64,
        ),
        (
            "KEYC_TYPE_SECONDCLICK",
            rmux_util::key::KeyCodeType::Secondclick as i64,
        ),
        (
            "KEYC_TYPE_DOUBLECLICK",
            rmux_util::key::KeyCodeType::Doubleclick as i64,
        ),
        (
            "KEYC_TYPE_TRIPLECLICK",
            rmux_util::key::KeyCodeType::Tripleclick as i64,
        ),
        (
            "KEYC_TYPE_NOTYPE",
            rmux_util::key::KeyCodeType::Notype as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_PANE",
            rmux_util::key::MouseLocation::Pane as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_STATUS",
            rmux_util::key::MouseLocation::Status as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_STATUS_LEFT",
            rmux_util::key::MouseLocation::StatusLeft as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_STATUS_RIGHT",
            rmux_util::key::MouseLocation::StatusRight as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_STATUS_DEFAULT",
            rmux_util::key::MouseLocation::StatusDefault as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_BORDER",
            rmux_util::key::MouseLocation::Border as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_SCROLLBAR_UP",
            rmux_util::key::MouseLocation::ScrollbarUp as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_SCROLLBAR_SLIDER",
            rmux_util::key::MouseLocation::ScrollbarSlider as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_SCROLLBAR_DOWN",
            rmux_util::key::MouseLocation::ScrollbarDown as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_EMPTY",
            rmux_util::key::MouseLocation::Empty as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL0",
            rmux_util::key::MouseLocation::Control0 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL1",
            rmux_util::key::MouseLocation::Control1 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL2",
            rmux_util::key::MouseLocation::Control2 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL3",
            rmux_util::key::MouseLocation::Control3 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL4",
            rmux_util::key::MouseLocation::Control4 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL5",
            rmux_util::key::MouseLocation::Control5 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL6",
            rmux_util::key::MouseLocation::Control6 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL7",
            rmux_util::key::MouseLocation::Control7 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL8",
            rmux_util::key::MouseLocation::Control8 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_CONTROL9",
            rmux_util::key::MouseLocation::Control9 as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_NOWHERE",
            rmux_util::key::MouseLocation::Nowhere as i64,
        ),
        (
            "COLOUR_THEME_BLACK",
            rmux_emu::colour::ColourTheme::Black as i64,
        ),
        (
            "COLOUR_THEME_WHITE",
            rmux_emu::colour::ColourTheme::White as i64,
        ),
        (
            "COLOUR_THEME_LIGHT_GREY",
            rmux_emu::colour::ColourTheme::LightGrey as i64,
        ),
        (
            "COLOUR_THEME_DARK_GREY",
            rmux_emu::colour::ColourTheme::DarkGrey as i64,
        ),
        (
            "COLOUR_THEME_GREEN",
            rmux_emu::colour::ColourTheme::Green as i64,
        ),
        (
            "COLOUR_THEME_YELLOW",
            rmux_emu::colour::ColourTheme::Yellow as i64,
        ),
        (
            "COLOUR_THEME_RED",
            rmux_emu::colour::ColourTheme::Red as i64,
        ),
        (
            "COLOUR_THEME_BLUE",
            rmux_emu::colour::ColourTheme::Blue as i64,
        ),
        (
            "COLOUR_THEME_CYAN",
            rmux_emu::colour::ColourTheme::Cyan as i64,
        ),
        (
            "STYLE_ALIGN_DEFAULT",
            rmux_emu::style::StyleAlign::Default as i64,
        ),
        ("STYLE_ALIGN_LEFT", rmux_emu::style::StyleAlign::Left as i64),
        (
            "STYLE_ALIGN_CENTRE",
            rmux_emu::style::StyleAlign::Centre as i64,
        ),
        (
            "STYLE_ALIGN_RIGHT",
            rmux_emu::style::StyleAlign::Right as i64,
        ),
        ("STYLE_LIST_OFF", rmux_emu::style::StyleList::Off as i64),
        ("STYLE_LIST_ON", rmux_emu::style::StyleList::On as i64),
        ("STYLE_LIST_FOCUS", rmux_emu::style::StyleList::Focus as i64),
        (
            "STYLE_LIST_LEFT_MARKER",
            rmux_emu::style::StyleList::LeftMarker as i64,
        ),
        (
            "STYLE_LIST_RIGHT_MARKER",
            rmux_emu::style::StyleList::RightMarker as i64,
        ),
        (
            "STYLE_RANGE_NONE",
            rmux_emu::style::StyleRangeType::None as i64,
        ),
        (
            "STYLE_RANGE_LEFT",
            rmux_emu::style::StyleRangeType::Left as i64,
        ),
        (
            "STYLE_RANGE_RIGHT",
            rmux_emu::style::StyleRangeType::Right as i64,
        ),
        (
            "STYLE_RANGE_PANE",
            rmux_emu::style::StyleRangeType::Pane as i64,
        ),
        (
            "STYLE_RANGE_WINDOW",
            rmux_emu::style::StyleRangeType::Window as i64,
        ),
        (
            "STYLE_RANGE_SESSION",
            rmux_emu::style::StyleRangeType::Session as i64,
        ),
        (
            "STYLE_RANGE_USER",
            rmux_emu::style::StyleRangeType::User as i64,
        ),
        (
            "STYLE_DEFAULT_BASE",
            rmux_emu::style::StyleDefaultType::Base as i64,
        ),
        (
            "STYLE_DEFAULT_PUSH",
            rmux_emu::style::StyleDefaultType::Push as i64,
        ),
        (
            "STYLE_DEFAULT_POP",
            rmux_emu::style::StyleDefaultType::Pop as i64,
        ),
        (
            "SCREEN_CURSOR_DEFAULT",
            rmux_emu::screen::ScreenCursorStyle::Default as i64,
        ),
        (
            "SCREEN_CURSOR_BLOCK",
            rmux_emu::screen::ScreenCursorStyle::Block as i64,
        ),
        (
            "SCREEN_CURSOR_UNDERLINE",
            rmux_emu::screen::ScreenCursorStyle::Underline as i64,
        ),
        (
            "PROGRESS_BAR_HIDDEN",
            rmux_emu::screen::ProgressBarState::Hidden as i64,
        ),
        (
            "PROGRESS_BAR_NORMAL",
            rmux_emu::screen::ProgressBarState::Normal as i64,
        ),
        (
            "PROGRESS_BAR_ERROR",
            rmux_emu::screen::ProgressBarState::Error as i64,
        ),
        (
            "PROGRESS_BAR_INDETERMINATE",
            rmux_emu::screen::ProgressBarState::Indeterminate as i64,
        ),
        (
            "PROGRESS_BAR_PAUSED",
            rmux_emu::screen::ProgressBarState::Paused as i64,
        ),
        (
            "BOX_LINES_DEFAULT",
            rmux_emu::screen::BoxLines::Default as i64,
        ),
        (
            "BOX_LINES_SINGLE",
            rmux_emu::screen::BoxLines::Single as i64,
        ),
        (
            "BOX_LINES_DOUBLE",
            rmux_emu::screen::BoxLines::Double as i64,
        ),
        ("BOX_LINES_HEAVY", rmux_emu::screen::BoxLines::Heavy as i64),
        (
            "BOX_LINES_SIMPLE",
            rmux_emu::screen::BoxLines::Simple as i64,
        ),
        (
            "BOX_LINES_ROUNDED",
            rmux_emu::screen::BoxLines::Rounded as i64,
        ),
        (
            "BOX_LINES_PADDED",
            rmux_emu::screen::BoxLines::Padded as i64,
        ),
        (
            "PANE_LINES_SINGLE",
            rmux_emu::screen::PaneLines::Single as i64,
        ),
        (
            "PANE_LINES_DOUBLE",
            rmux_emu::screen::PaneLines::Double as i64,
        ),
        (
            "PANE_LINES_HEAVY",
            rmux_emu::screen::PaneLines::Heavy as i64,
        ),
        (
            "PANE_LINES_SIMPLE",
            rmux_emu::screen::PaneLines::Simple as i64,
        ),
        (
            "PANE_LINES_NUMBER",
            rmux_emu::screen::PaneLines::Number as i64,
        ),
        (
            "PANE_LINES_SPACES",
            rmux_emu::screen::PaneLines::Spaces as i64,
        ),
        ("PANE_LINES_NONE", rmux_emu::screen::PaneLines::None as i64),
        (
            "INPUT_REQUEST_PALETTE",
            rmux_emu::input::InputRequestType::Palette as i64,
        ),
        (
            "INPUT_REQUEST_CLIPBOARD",
            rmux_emu::input::InputRequestType::Clipboard as i64,
        ),
        (
            "THEME_UNKNOWN",
            rmux_emu::colour::ClientTheme::Unknown as i64,
        ),
        ("THEME_LIGHT", rmux_emu::colour::ClientTheme::Light as i64),
        (
            "LAYOUT_LEFTRIGHT",
            rmux_server::layout::LayoutType::Leftright as i64,
        ),
        (
            "LAYOUT_TOPBOTTOM",
            rmux_server::layout::LayoutType::Topbottom as i64,
        ),
        (
            "ARGS_NONE",
            rmux_server::cmd::arguments::ArgsType::None as i64,
        ),
        (
            "ARGS_STRING",
            rmux_server::cmd::arguments::ArgsType::String as i64,
        ),
        (
            "ARGS_PARSE_INVALID",
            rmux_server::cmd::arguments::ArgsParseType::Invalid as i64,
        ),
        (
            "ARGS_PARSE_STRING",
            rmux_server::cmd::arguments::ArgsParseType::String as i64,
        ),
        (
            "ARGS_PARSE_COMMANDS_OR_STRING",
            rmux_server::cmd::arguments::ArgsParseType::CommandsOrString as i64,
        ),
        (
            "CMD_FIND_PANE",
            rmux_server::cmd::find::CmdFindType::Pane as i64,
        ),
        (
            "CMD_FIND_WINDOW",
            rmux_server::cmd::find::CmdFindType::Window as i64,
        ),
        (
            "CMD_FIND_SESSION",
            rmux_server::cmd::find::CmdFindType::Session as i64,
        ),
        (
            "CMD_RETURN_ERROR",
            rmux_server::cmd::queue::CmdReturn::Error as i64,
        ),
        (
            "CMD_RETURN_NORMAL",
            rmux_server::cmd::queue::CmdReturn::Normal as i64,
        ),
        (
            "CMD_RETURN_WAIT",
            rmux_server::cmd::queue::CmdReturn::Wait as i64,
        ),
        (
            "CMD_PARSE_ERROR",
            rmux_server::cmd::parse::CmdParseStatus::Error as i64,
        ),
        (
            "PROMPT_TYPE_COMMAND",
            rmux_server::ui::prompt::PromptType::Command as i64,
        ),
        (
            "PROMPT_TYPE_SEARCH",
            rmux_server::ui::prompt::PromptType::Search as i64,
        ),
        (
            "PROMPT_TYPE_INVALID",
            rmux_server::ui::prompt::PromptType::Invalid as i64,
        ),
        (
            "PROMPT_CONTINUE",
            rmux_server::ui::prompt::PromptResult::Continue as i64,
        ),
        (
            "PROMPT_KEY_NOT_HANDLED",
            rmux_server::ui::prompt::PromptKeyResult::NotHandled as i64,
        ),
        (
            "PROMPT_KEY_HANDLED",
            rmux_server::ui::prompt::PromptKeyResult::Handled as i64,
        ),
        (
            "PROMPT_KEY_CLOSE",
            rmux_server::ui::prompt::PromptKeyResult::Close as i64,
        ),
        (
            "MONITOR_SESSION",
            rmux_server::model::monitor::MonitorType::Session as i64,
        ),
        (
            "MONITOR_PANE",
            rmux_server::model::monitor::MonitorType::Pane as i64,
        ),
        (
            "MONITOR_ALL_PANES",
            rmux_server::model::monitor::MonitorType::AllPanes as i64,
        ),
        (
            "MONITOR_WINDOW",
            rmux_server::model::monitor::MonitorType::Window as i64,
        ),
        (
            "EVENT_PAYLOAD_STRING",
            rmux_server::server::events::EventPayloadType::String as i64,
        ),
        (
            "EVENT_PAYLOAD_TIME",
            rmux_server::server::events::EventPayloadType::Time as i64,
        ),
        (
            "EVENT_PAYLOAD_INT",
            rmux_server::server::events::EventPayloadType::Int as i64,
        ),
        (
            "EVENT_PAYLOAD_UINT",
            rmux_server::server::events::EventPayloadType::Uint as i64,
        ),
        (
            "EVENT_PAYLOAD_CLIENT",
            rmux_server::server::events::EventPayloadType::Client as i64,
        ),
        (
            "EVENT_PAYLOAD_SESSION",
            rmux_server::server::events::EventPayloadType::Session as i64,
        ),
        (
            "EVENT_PAYLOAD_WINDOW",
            rmux_server::server::events::EventPayloadType::Window as i64,
        ),
        (
            "EVENT_PAYLOAD_PANE",
            rmux_server::server::events::EventPayloadType::Pane as i64,
        ),
        (
            "OPTIONS_TABLE_STRING",
            rmux_server::options::OptionsTableType::String as i64,
        ),
        (
            "OPTIONS_TABLE_NUMBER",
            rmux_server::options::OptionsTableType::Number as i64,
        ),
        (
            "OPTIONS_TABLE_KEY",
            rmux_server::options::OptionsTableType::Key as i64,
        ),
        (
            "OPTIONS_TABLE_COLOUR",
            rmux_server::options::OptionsTableType::Colour as i64,
        ),
        (
            "OPTIONS_TABLE_FLAG",
            rmux_server::options::OptionsTableType::Flag as i64,
        ),
        (
            "OPTIONS_TABLE_CHOICE",
            rmux_server::options::OptionsTableType::Choice as i64,
        ),
        (
            "SORT_ACTIVITY",
            rmux_server::format::sort::SortOrder::Activity as i64,
        ),
        (
            "SORT_CREATION",
            rmux_server::format::sort::SortOrder::Creation as i64,
        ),
        (
            "SORT_INDEX",
            rmux_server::format::sort::SortOrder::Index as i64,
        ),
        (
            "SORT_MODIFIER",
            rmux_server::format::sort::SortOrder::Modifier as i64,
        ),
        (
            "SORT_NAME",
            rmux_server::format::sort::SortOrder::Name as i64,
        ),
        (
            "SORT_ORDER",
            rmux_server::format::sort::SortOrder::Order as i64,
        ),
        (
            "SORT_SIZE",
            rmux_server::format::sort::SortOrder::Size as i64,
        ),
        ("SORT_Z", rmux_server::format::sort::SortOrder::Z as i64),
        ("SORT_END", rmux_server::format::sort::SortOrder::End as i64),
        ("C0_NUL", rmux_util::key::C0::NUL as i64),
        ("C0_SOH", rmux_util::key::C0::SOH as i64),
        ("C0_STX", rmux_util::key::C0::STX as i64),
        ("C0_ETX", rmux_util::key::C0::ETX as i64),
        ("C0_EOT", rmux_util::key::C0::EOT as i64),
        ("C0_ENQ", rmux_util::key::C0::ENQ as i64),
        ("C0_ASC", rmux_util::key::C0::ASC as i64),
        ("C0_BEL", rmux_util::key::C0::BEL as i64),
        ("C0_BS", rmux_util::key::C0::BS as i64),
        ("C0_HT", rmux_util::key::C0::HT as i64),
        ("C0_LF", rmux_util::key::C0::LF as i64),
        ("C0_VT", rmux_util::key::C0::VT as i64),
        ("C0_FF", rmux_util::key::C0::FF as i64),
        ("C0_CR", rmux_util::key::C0::CR as i64),
        ("C0_SO", rmux_util::key::C0::SO as i64),
        ("C0_SI", rmux_util::key::C0::SI as i64),
        ("C0_DLE", rmux_util::key::C0::DLE as i64),
        ("C0_DC1", rmux_util::key::C0::DC1 as i64),
        ("C0_DC2", rmux_util::key::C0::DC2 as i64),
        ("C0_DC3", rmux_util::key::C0::DC3 as i64),
        ("C0_DC4", rmux_util::key::C0::DC4 as i64),
        ("C0_NAK", rmux_util::key::C0::NAK as i64),
        ("C0_SYN", rmux_util::key::C0::SYN as i64),
        ("C0_ETB", rmux_util::key::C0::ETB as i64),
        ("C0_CAN", rmux_util::key::C0::CAN as i64),
        ("C0_EM", rmux_util::key::C0::EM as i64),
        ("C0_SUB", rmux_util::key::C0::SUB as i64),
        ("C0_ESC", rmux_util::key::C0::ESC as i64),
        ("C0_FS", rmux_util::key::C0::FS as i64),
        ("C0_GS", rmux_util::key::C0::GS as i64),
        ("C0_RS", rmux_util::key::C0::RS as i64),
        ("KEYC_USER", rmux_util::key::SpecialKey::USER as i64),
        ("KEYC_NONE", rmux_util::key::SpecialKey::NONE as i64),
        ("KEYC_UNKNOWN", rmux_util::key::SpecialKey::UNKNOWN as i64),
        ("KEYC_FOCUS_IN", rmux_util::key::SpecialKey::FOCUS_IN as i64),
        (
            "KEYC_FOCUS_OUT",
            rmux_util::key::SpecialKey::FOCUS_OUT as i64,
        ),
        ("KEYC_ANY", rmux_util::key::SpecialKey::ANY as i64),
        (
            "KEYC_PASTE_START",
            rmux_util::key::SpecialKey::PASTE_START as i64,
        ),
        (
            "KEYC_PASTE_END",
            rmux_util::key::SpecialKey::PASTE_END as i64,
        ),
        ("KEYC_BSPACE", rmux_util::key::SpecialKey::BSPACE as i64),
        ("KEYC_F1", rmux_util::key::SpecialKey::F1 as i64),
        ("KEYC_F2", rmux_util::key::SpecialKey::F2 as i64),
        ("KEYC_F3", rmux_util::key::SpecialKey::F3 as i64),
        ("KEYC_F4", rmux_util::key::SpecialKey::F4 as i64),
        ("KEYC_F5", rmux_util::key::SpecialKey::F5 as i64),
        ("KEYC_F6", rmux_util::key::SpecialKey::F6 as i64),
        ("KEYC_F7", rmux_util::key::SpecialKey::F7 as i64),
        ("KEYC_F8", rmux_util::key::SpecialKey::F8 as i64),
        ("KEYC_F9", rmux_util::key::SpecialKey::F9 as i64),
        ("KEYC_F10", rmux_util::key::SpecialKey::F10 as i64),
        ("KEYC_F11", rmux_util::key::SpecialKey::F11 as i64),
        ("KEYC_F12", rmux_util::key::SpecialKey::F12 as i64),
        ("KEYC_IC", rmux_util::key::SpecialKey::IC as i64),
        ("KEYC_DC", rmux_util::key::SpecialKey::DC as i64),
        ("KEYC_HOME", rmux_util::key::SpecialKey::HOME as i64),
        ("KEYC_END", rmux_util::key::SpecialKey::END as i64),
        ("KEYC_NPAGE", rmux_util::key::SpecialKey::NPAGE as i64),
        ("KEYC_PPAGE", rmux_util::key::SpecialKey::PPAGE as i64),
        ("KEYC_BTAB", rmux_util::key::SpecialKey::BTAB as i64),
        ("KEYC_UP", rmux_util::key::SpecialKey::UP as i64),
        ("KEYC_DOWN", rmux_util::key::SpecialKey::DOWN as i64),
        ("KEYC_LEFT", rmux_util::key::SpecialKey::LEFT as i64),
        ("KEYC_RIGHT", rmux_util::key::SpecialKey::RIGHT as i64),
        ("KEYC_KP_SLASH", rmux_util::key::SpecialKey::KP_SLASH as i64),
        ("KEYC_KP_STAR", rmux_util::key::SpecialKey::KP_STAR as i64),
        ("KEYC_KP_MINUS", rmux_util::key::SpecialKey::KP_MINUS as i64),
        ("KEYC_KP_SEVEN", rmux_util::key::SpecialKey::KP_SEVEN as i64),
        ("KEYC_KP_EIGHT", rmux_util::key::SpecialKey::KP_EIGHT as i64),
        ("KEYC_KP_NINE", rmux_util::key::SpecialKey::KP_NINE as i64),
        ("KEYC_KP_PLUS", rmux_util::key::SpecialKey::KP_PLUS as i64),
        ("KEYC_KP_FOUR", rmux_util::key::SpecialKey::KP_FOUR as i64),
        ("KEYC_KP_FIVE", rmux_util::key::SpecialKey::KP_FIVE as i64),
        ("KEYC_KP_SIX", rmux_util::key::SpecialKey::KP_SIX as i64),
        ("KEYC_KP_ONE", rmux_util::key::SpecialKey::KP_ONE as i64),
        ("KEYC_KP_TWO", rmux_util::key::SpecialKey::KP_TWO as i64),
        ("KEYC_KP_THREE", rmux_util::key::SpecialKey::KP_THREE as i64),
        ("KEYC_KP_ENTER", rmux_util::key::SpecialKey::KP_ENTER as i64),
        ("KEYC_KP_ZERO", rmux_util::key::SpecialKey::KP_ZERO as i64),
        (
            "KEYC_KP_PERIOD",
            rmux_util::key::SpecialKey::KP_PERIOD as i64,
        ),
        (
            "KEYC_REPORT_DARK_THEME",
            rmux_util::key::SpecialKey::REPORT_DARK_THEME as i64,
        ),
        (
            "KEYC_REPORT_LIGHT_THEME",
            rmux_util::key::SpecialKey::REPORT_LIGHT_THEME as i64,
        ),
        ("KEYC_MOUSE", rmux_util::key::SpecialKey::MOUSE as i64),
        ("KEYC_DRAGGING", rmux_util::key::SpecialKey::DRAGGING as i64),
        (
            "KEYC_DOUBLECLICK",
            rmux_util::key::SpecialKey::DOUBLECLICK as i64,
        ),
        (
            "KEYC_MOUSEMOVE_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE1_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE2_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE3_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE6_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE7_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE8_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE9_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE10_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_PANE",
            rmux_util::key::SpecialKey::MOUSEMOVE11_PANE as i64,
        ),
        (
            "KEYC_MOUSEMOVE_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE1_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE2_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE3_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE6_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE7_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE8_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE9_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE10_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_STATUS",
            rmux_util::key::SpecialKey::MOUSEMOVE11_STATUS as i64,
        ),
        (
            "KEYC_MOUSEMOVE_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEMOVE11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEMOVE_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEMOVE11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEMOVE_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEMOVE11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEMOVE_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE1_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE2_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE3_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE6_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE7_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE8_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE9_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE10_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_BORDER",
            rmux_util::key::SpecialKey::MOUSEMOVE11_BORDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEMOVE11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEMOVE_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEMOVE11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEMOVE_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEMOVE11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEMOVE_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE1_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE2_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE3_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE6_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE7_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE8_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE9_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE10_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_EMPTY",
            rmux_util::key::SpecialKey::MOUSEMOVE11_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEMOVE_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE1_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE1_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE2_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE2_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE3_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE3_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE6_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE6_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE7_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE7_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE8_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE8_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE9_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE9_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE10_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE10_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEMOVE11_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEMOVE11_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN1_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN1_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN2_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN2_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN3_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN3_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN6_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN6_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN7_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN7_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN8_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN8_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN9_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN9_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN10_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN10_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN11_PANE",
            rmux_util::key::SpecialKey::WHEELDOWN11_PANE as i64,
        ),
        (
            "KEYC_WHEELDOWN_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN1_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN1_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN2_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN2_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN3_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN3_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN6_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN6_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN7_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN7_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN8_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN8_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN9_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN9_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN10_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN10_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN11_STATUS",
            rmux_util::key::SpecialKey::WHEELDOWN11_STATUS as i64,
        ),
        (
            "KEYC_WHEELDOWN_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN1_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN2_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN3_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN6_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN7_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN8_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN9_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN10_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN11_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELDOWN11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELDOWN_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELDOWN11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELDOWN_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELDOWN11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELDOWN_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN1_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN1_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN2_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN2_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN3_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN3_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN6_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN6_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN7_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN7_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN8_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN8_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN9_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN9_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN10_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN10_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN11_BORDER",
            rmux_util::key::SpecialKey::WHEELDOWN11_BORDER as i64,
        ),
        (
            "KEYC_WHEELDOWN_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELDOWN11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELDOWN_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELDOWN11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELDOWN_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELDOWN11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELDOWN_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN1_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN1_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN2_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN2_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN3_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN3_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN6_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN6_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN7_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN7_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN8_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN8_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN9_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN9_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN10_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN10_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN11_EMPTY",
            rmux_util::key::SpecialKey::WHEELDOWN11_EMPTY as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL0",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL1",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL2",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL3",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL4",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL5",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL6",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL7",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL8",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELDOWN_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN1_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN1_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN2_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN2_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN3_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN3_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN6_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN6_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN7_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN7_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN8_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN8_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN9_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN9_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN10_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN10_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELDOWN11_CONTROL9",
            rmux_util::key::SpecialKey::WHEELDOWN11_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP_PANE",
            rmux_util::key::SpecialKey::WHEELUP_PANE as i64,
        ),
        (
            "KEYC_WHEELUP1_PANE",
            rmux_util::key::SpecialKey::WHEELUP1_PANE as i64,
        ),
        (
            "KEYC_WHEELUP2_PANE",
            rmux_util::key::SpecialKey::WHEELUP2_PANE as i64,
        ),
        (
            "KEYC_WHEELUP3_PANE",
            rmux_util::key::SpecialKey::WHEELUP3_PANE as i64,
        ),
        (
            "KEYC_WHEELUP6_PANE",
            rmux_util::key::SpecialKey::WHEELUP6_PANE as i64,
        ),
        (
            "KEYC_WHEELUP7_PANE",
            rmux_util::key::SpecialKey::WHEELUP7_PANE as i64,
        ),
        (
            "KEYC_WHEELUP8_PANE",
            rmux_util::key::SpecialKey::WHEELUP8_PANE as i64,
        ),
        (
            "KEYC_WHEELUP9_PANE",
            rmux_util::key::SpecialKey::WHEELUP9_PANE as i64,
        ),
        (
            "KEYC_WHEELUP10_PANE",
            rmux_util::key::SpecialKey::WHEELUP10_PANE as i64,
        ),
        (
            "KEYC_WHEELUP11_PANE",
            rmux_util::key::SpecialKey::WHEELUP11_PANE as i64,
        ),
        (
            "KEYC_WHEELUP_STATUS",
            rmux_util::key::SpecialKey::WHEELUP_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP1_STATUS",
            rmux_util::key::SpecialKey::WHEELUP1_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP2_STATUS",
            rmux_util::key::SpecialKey::WHEELUP2_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP3_STATUS",
            rmux_util::key::SpecialKey::WHEELUP3_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP6_STATUS",
            rmux_util::key::SpecialKey::WHEELUP6_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP7_STATUS",
            rmux_util::key::SpecialKey::WHEELUP7_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP8_STATUS",
            rmux_util::key::SpecialKey::WHEELUP8_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP9_STATUS",
            rmux_util::key::SpecialKey::WHEELUP9_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP10_STATUS",
            rmux_util::key::SpecialKey::WHEELUP10_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP11_STATUS",
            rmux_util::key::SpecialKey::WHEELUP11_STATUS as i64,
        ),
        (
            "KEYC_WHEELUP_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP1_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP2_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP3_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP6_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP7_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP8_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP9_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP10_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP11_STATUS_LEFT",
            rmux_util::key::SpecialKey::WHEELUP11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_WHEELUP_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::WHEELUP11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_WHEELUP_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::WHEELUP11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_WHEELUP_BORDER",
            rmux_util::key::SpecialKey::WHEELUP_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP1_BORDER",
            rmux_util::key::SpecialKey::WHEELUP1_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP2_BORDER",
            rmux_util::key::SpecialKey::WHEELUP2_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP3_BORDER",
            rmux_util::key::SpecialKey::WHEELUP3_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP6_BORDER",
            rmux_util::key::SpecialKey::WHEELUP6_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP7_BORDER",
            rmux_util::key::SpecialKey::WHEELUP7_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP8_BORDER",
            rmux_util::key::SpecialKey::WHEELUP8_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP9_BORDER",
            rmux_util::key::SpecialKey::WHEELUP9_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP10_BORDER",
            rmux_util::key::SpecialKey::WHEELUP10_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP11_BORDER",
            rmux_util::key::SpecialKey::WHEELUP11_BORDER as i64,
        ),
        (
            "KEYC_WHEELUP_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::WHEELUP11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_WHEELUP_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::WHEELUP11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_WHEELUP_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::WHEELUP11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_WHEELUP_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP1_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP1_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP2_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP2_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP3_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP3_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP6_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP6_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP7_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP7_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP8_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP8_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP9_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP9_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP10_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP10_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP11_EMPTY",
            rmux_util::key::SpecialKey::WHEELUP11_EMPTY as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL0",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL0 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL1",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL1 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL2",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL2 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL3",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL3 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL4",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL4 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL5",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL5 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL6",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL6 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL7",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL7 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL8",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL8 as i64,
        ),
        (
            "KEYC_WHEELUP_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP1_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP1_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP2_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP2_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP3_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP3_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP6_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP6_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP7_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP7_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP8_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP8_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP9_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP9_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP10_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP10_CONTROL9 as i64,
        ),
        (
            "KEYC_WHEELUP11_CONTROL9",
            rmux_util::key::SpecialKey::WHEELUP11_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN1_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN2_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN3_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN6_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN7_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN8_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN9_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN10_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_PANE",
            rmux_util::key::SpecialKey::MOUSEDOWN11_PANE as i64,
        ),
        (
            "KEYC_MOUSEDOWN_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN1_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN2_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN3_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN6_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN7_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN8_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN9_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN10_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_STATUS",
            rmux_util::key::SpecialKey::MOUSEDOWN11_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDOWN_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDOWN11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDOWN_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDOWN11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDOWN_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDOWN11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDOWN_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN1_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN2_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN3_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN6_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN7_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN8_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN9_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN10_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_BORDER",
            rmux_util::key::SpecialKey::MOUSEDOWN11_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDOWN11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDOWN_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDOWN11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDOWN_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDOWN11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDOWN_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN1_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN2_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN3_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN6_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN7_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN8_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN9_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN10_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDOWN11_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDOWN_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN1_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN1_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN2_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN2_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN3_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN3_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN6_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN6_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN7_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN7_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN8_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN8_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN9_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN9_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN10_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN10_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDOWN11_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDOWN11_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP_PANE",
            rmux_util::key::SpecialKey::MOUSEUP_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP1_PANE",
            rmux_util::key::SpecialKey::MOUSEUP1_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP2_PANE",
            rmux_util::key::SpecialKey::MOUSEUP2_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP3_PANE",
            rmux_util::key::SpecialKey::MOUSEUP3_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP6_PANE",
            rmux_util::key::SpecialKey::MOUSEUP6_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP7_PANE",
            rmux_util::key::SpecialKey::MOUSEUP7_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP8_PANE",
            rmux_util::key::SpecialKey::MOUSEUP8_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP9_PANE",
            rmux_util::key::SpecialKey::MOUSEUP9_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP10_PANE",
            rmux_util::key::SpecialKey::MOUSEUP10_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP11_PANE",
            rmux_util::key::SpecialKey::MOUSEUP11_PANE as i64,
        ),
        (
            "KEYC_MOUSEUP_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP1_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP1_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP2_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP2_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP3_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP3_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP6_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP6_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP7_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP7_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP8_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP8_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP9_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP9_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP10_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP10_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP11_STATUS",
            rmux_util::key::SpecialKey::MOUSEUP11_STATUS as i64,
        ),
        (
            "KEYC_MOUSEUP_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP1_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP2_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP3_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP6_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP7_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP8_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP9_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP10_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP11_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEUP11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEUP_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEUP11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEUP_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEUP11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEUP_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP1_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP1_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP2_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP2_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP3_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP3_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP6_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP6_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP7_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP7_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP8_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP8_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP9_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP9_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP10_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP10_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP11_BORDER",
            rmux_util::key::SpecialKey::MOUSEUP11_BORDER as i64,
        ),
        (
            "KEYC_MOUSEUP_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEUP11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEUP_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEUP11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEUP_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEUP11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEUP_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP1_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP1_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP2_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP2_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP3_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP3_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP6_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP6_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP7_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP7_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP8_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP8_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP9_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP9_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP10_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP10_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP11_EMPTY",
            rmux_util::key::SpecialKey::MOUSEUP11_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEUP_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP1_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP1_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP2_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP2_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP3_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP3_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP6_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP6_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP7_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP7_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP8_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP8_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP9_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP9_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP10_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP10_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEUP11_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEUP11_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG1_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG2_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG3_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG6_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG7_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG8_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG9_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG10_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAG11_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAG_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG1_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG2_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG3_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG6_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG7_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG8_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG9_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG10_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAG11_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAG_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAG11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAG_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAG11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAG_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAG11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAG_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG1_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG2_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG3_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG6_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG7_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG8_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG9_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG10_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAG11_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAG11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAG_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAG11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAG_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAG11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAG_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG1_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG2_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG3_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG6_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG7_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG8_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG9_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG10_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAG11_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAG_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG1_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG1_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG2_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG2_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG3_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG3_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG6_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG6_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG7_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG7_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG8_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG8_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG9_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG9_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG10_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG10_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAG11_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAG11_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_PANE",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_PANE as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_STATUS",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_STATUS as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_STATUS_LEFT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_BORDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_BORDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_EMPTY",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_EMPTY as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL0",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL0 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL1",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL1 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL2",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL2 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL3",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL3 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL4",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL4 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL5",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL5 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL6",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL6 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL7",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL7 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL8",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL8 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND1_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND1_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND2_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND2_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND3_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND3_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND6_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND6_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND7_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND7_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND8_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND8_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND9_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND9_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND10_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND10_CONTROL9 as i64,
        ),
        (
            "KEYC_MOUSEDRAGEND11_CONTROL9",
            rmux_util::key::SpecialKey::MOUSEDRAGEND11_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK1_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK1_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK2_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK2_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK3_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK3_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK6_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK6_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK7_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK7_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK8_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK8_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK9_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK9_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK10_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK10_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK11_PANE",
            rmux_util::key::SpecialKey::SECONDCLICK11_PANE as i64,
        ),
        (
            "KEYC_SECONDCLICK_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK1_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK1_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK2_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK2_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK3_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK3_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK6_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK6_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK7_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK7_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK8_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK8_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK9_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK9_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK10_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK10_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK11_STATUS",
            rmux_util::key::SpecialKey::SECONDCLICK11_STATUS as i64,
        ),
        (
            "KEYC_SECONDCLICK_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK1_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK2_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK3_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK6_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK7_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK8_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK9_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK10_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK11_STATUS_LEFT",
            rmux_util::key::SpecialKey::SECONDCLICK11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_SECONDCLICK_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::SECONDCLICK11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_SECONDCLICK_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::SECONDCLICK11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_SECONDCLICK_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK1_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK1_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK2_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK2_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK3_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK3_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK6_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK6_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK7_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK7_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK8_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK8_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK9_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK9_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK10_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK10_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK11_BORDER",
            rmux_util::key::SpecialKey::SECONDCLICK11_BORDER as i64,
        ),
        (
            "KEYC_SECONDCLICK_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::SECONDCLICK11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_SECONDCLICK_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::SECONDCLICK11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_SECONDCLICK_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::SECONDCLICK11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_SECONDCLICK_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK1_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK1_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK2_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK2_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK3_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK3_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK6_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK6_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK7_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK7_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK8_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK8_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK9_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK9_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK10_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK10_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK11_EMPTY",
            rmux_util::key::SpecialKey::SECONDCLICK11_EMPTY as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL0",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL0 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL1",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL1 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL2",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL2 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL3",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL3 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL4",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL4 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL5",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL5 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL6",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL6 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL7",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL7 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL8",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL8 as i64,
        ),
        (
            "KEYC_SECONDCLICK_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK1_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK1_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK2_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK2_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK3_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK3_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK6_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK6_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK7_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK7_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK8_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK8_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK9_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK9_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK10_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK10_CONTROL9 as i64,
        ),
        (
            "KEYC_SECONDCLICK11_CONTROL9",
            rmux_util::key::SpecialKey::SECONDCLICK11_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK1_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK2_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK3_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK6_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK7_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK8_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK9_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK10_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_PANE",
            rmux_util::key::SpecialKey::DOUBLECLICK11_PANE as i64,
        ),
        (
            "KEYC_DOUBLECLICK_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK1_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK2_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK3_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK6_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK7_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK8_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK9_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK10_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_STATUS",
            rmux_util::key::SpecialKey::DOUBLECLICK11_STATUS as i64,
        ),
        (
            "KEYC_DOUBLECLICK_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_STATUS_LEFT",
            rmux_util::key::SpecialKey::DOUBLECLICK11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_DOUBLECLICK_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::DOUBLECLICK11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_DOUBLECLICK_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::DOUBLECLICK11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_DOUBLECLICK_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK1_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK2_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK3_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK6_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK7_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK8_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK9_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK10_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_BORDER",
            rmux_util::key::SpecialKey::DOUBLECLICK11_BORDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::DOUBLECLICK11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_DOUBLECLICK_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::DOUBLECLICK11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_DOUBLECLICK_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::DOUBLECLICK11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_DOUBLECLICK_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK1_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK2_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK3_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK6_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK7_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK8_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK9_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK10_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_EMPTY",
            rmux_util::key::SpecialKey::DOUBLECLICK11_EMPTY as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL0",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL0 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL1",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL1 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL2",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL2 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL3",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL3 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL4",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL4 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL5",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL5 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL6",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL6 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL7",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL7 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL8",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL8 as i64,
        ),
        (
            "KEYC_DOUBLECLICK_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK1_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK1_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK2_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK2_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK3_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK3_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK6_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK6_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK7_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK7_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK8_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK8_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK9_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK9_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK10_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK10_CONTROL9 as i64,
        ),
        (
            "KEYC_DOUBLECLICK11_CONTROL9",
            rmux_util::key::SpecialKey::DOUBLECLICK11_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK1_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK2_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK3_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK6_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK7_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK8_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK9_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK10_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_PANE",
            rmux_util::key::SpecialKey::TRIPLECLICK11_PANE as i64,
        ),
        (
            "KEYC_TRIPLECLICK_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK1_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK2_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK3_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK6_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK7_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK8_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK9_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK10_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_STATUS",
            rmux_util::key::SpecialKey::TRIPLECLICK11_STATUS as i64,
        ),
        (
            "KEYC_TRIPLECLICK_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK1_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK2_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK3_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK6_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK7_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK8_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK9_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK10_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_STATUS_LEFT",
            rmux_util::key::SpecialKey::TRIPLECLICK11_STATUS_LEFT as i64,
        ),
        (
            "KEYC_TRIPLECLICK_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK1_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK2_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK3_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK6_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK7_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK8_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK9_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK10_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_STATUS_RIGHT",
            rmux_util::key::SpecialKey::TRIPLECLICK11_STATUS_RIGHT as i64,
        ),
        (
            "KEYC_TRIPLECLICK_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK1_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK2_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK3_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK6_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK7_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK8_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK9_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK10_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_STATUS_DEFAULT",
            rmux_util::key::SpecialKey::TRIPLECLICK11_STATUS_DEFAULT as i64,
        ),
        (
            "KEYC_TRIPLECLICK_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK1_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK2_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK3_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK6_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK7_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK8_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK9_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK10_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_BORDER",
            rmux_util::key::SpecialKey::TRIPLECLICK11_BORDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK1_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK2_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK3_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK6_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK7_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK8_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK9_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK10_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_SCROLLBAR_UP",
            rmux_util::key::SpecialKey::TRIPLECLICK11_SCROLLBAR_UP as i64,
        ),
        (
            "KEYC_TRIPLECLICK_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK1_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK2_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK3_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK6_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK7_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK8_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK9_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK10_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_SCROLLBAR_SLIDER",
            rmux_util::key::SpecialKey::TRIPLECLICK11_SCROLLBAR_SLIDER as i64,
        ),
        (
            "KEYC_TRIPLECLICK_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK1_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK2_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK3_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK6_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK7_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK8_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK9_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK10_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_SCROLLBAR_DOWN",
            rmux_util::key::SpecialKey::TRIPLECLICK11_SCROLLBAR_DOWN as i64,
        ),
        (
            "KEYC_TRIPLECLICK_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK1_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK2_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK3_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK6_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK7_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK8_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK9_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK10_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_EMPTY",
            rmux_util::key::SpecialKey::TRIPLECLICK11_EMPTY as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL0",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL0 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL1",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL1 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL2",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL2 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL3",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL3 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL4",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL4 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL5",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL5 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL6",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL6 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL7",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL7 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL8",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL8 as i64,
        ),
        (
            "KEYC_TRIPLECLICK_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK1_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK1_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK2_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK2_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK3_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK3_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK6_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK6_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK7_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK7_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK8_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK8_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK9_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK9_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK10_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK10_CONTROL9 as i64,
        ),
        (
            "KEYC_TRIPLECLICK11_CONTROL9",
            rmux_util::key::SpecialKey::TRIPLECLICK11_CONTROL9 as i64,
        ),
        (
            "CLIENT_EXIT_RETURN",
            rmux_server::client::ClientExitType::Return as i64,
        ),
        (
            "CLIENT_EXIT_SHUTDOWN",
            rmux_server::client::ClientExitType::Shutdown as i64,
        ),
        (
            "KEYC_META",
            rmux_util::key::KeyModifiers::META.bits() as i64,
        ),
        (
            "KEYC_CTRL",
            rmux_util::key::KeyModifiers::CTRL.bits() as i64,
        ),
        (
            "KEYC_SHIFT",
            rmux_util::key::KeyModifiers::SHIFT.bits() as i64,
        ),
        (
            "KEYC_LITERAL",
            rmux_util::key::KeyFlags::LITERAL.bits() as i64,
        ),
        (
            "KEYC_KEYPAD",
            rmux_util::key::KeyFlags::KEYPAD.bits() as i64,
        ),
        (
            "KEYC_CURSOR",
            rmux_util::key::KeyFlags::CURSOR.bits() as i64,
        ),
        (
            "KEYC_IMPLIED_META",
            rmux_util::key::KeyFlags::IMPLIED_META.bits() as i64,
        ),
        (
            "KEYC_BUILD_MODIFIERS",
            rmux_util::key::KeyFlags::BUILD_MODIFIERS.bits() as i64,
        ),
        ("KEYC_VI", rmux_util::key::KeyFlags::VI.bits() as i64),
        ("KEYC_SENT", rmux_util::key::KeyFlags::SENT.bits() as i64),
        ("KEYC_MASK_TYPE", rmux_util::key::KeyMasks::TYPE as i64),
        (
            "KEYC_MASK_MODIFIERS",
            rmux_util::key::KeyMasks::MODIFIERS as i64,
        ),
        ("KEYC_MASK_FLAGS", rmux_util::key::KeyMasks::FLAGS as i64),
        ("KEYC_MASK_KEY", rmux_util::key::KeyMasks::KEY as i64),
        (
            "MODE_CURSOR",
            rmux_emu::screen::ScreenMode::CURSOR.bits() as i64,
        ),
        (
            "MODE_INSERT",
            rmux_emu::screen::ScreenMode::INSERT.bits() as i64,
        ),
        (
            "MODE_KCURSOR",
            rmux_emu::screen::ScreenMode::KCURSOR.bits() as i64,
        ),
        (
            "MODE_KKEYPAD",
            rmux_emu::screen::ScreenMode::KKEYPAD.bits() as i64,
        ),
        (
            "MODE_WRAP",
            rmux_emu::screen::ScreenMode::WRAP.bits() as i64,
        ),
        (
            "MODE_MOUSE_STANDARD",
            rmux_emu::screen::ScreenMode::MOUSE_STANDARD.bits() as i64,
        ),
        (
            "MODE_MOUSE_BUTTON",
            rmux_emu::screen::ScreenMode::MOUSE_BUTTON.bits() as i64,
        ),
        (
            "MODE_CURSOR_BLINKING",
            rmux_emu::screen::ScreenMode::CURSOR_BLINKING.bits() as i64,
        ),
        (
            "MODE_MOUSE_UTF8",
            rmux_emu::screen::ScreenMode::MOUSE_UTF8.bits() as i64,
        ),
        (
            "MODE_MOUSE_SGR",
            rmux_emu::screen::ScreenMode::MOUSE_SGR.bits() as i64,
        ),
        (
            "MODE_BRACKETPASTE",
            rmux_emu::screen::ScreenMode::BRACKETPASTE.bits() as i64,
        ),
        (
            "MODE_FOCUSON",
            rmux_emu::screen::ScreenMode::FOCUSON.bits() as i64,
        ),
        (
            "MODE_MOUSE_ALL",
            rmux_emu::screen::ScreenMode::MOUSE_ALL.bits() as i64,
        ),
        (
            "MODE_ORIGIN",
            rmux_emu::screen::ScreenMode::ORIGIN.bits() as i64,
        ),
        (
            "MODE_CRLF",
            rmux_emu::screen::ScreenMode::CRLF.bits() as i64,
        ),
        (
            "MODE_KEYS_EXTENDED",
            rmux_emu::screen::ScreenMode::KEYS_EXTENDED.bits() as i64,
        ),
        (
            "MODE_CURSOR_VERY_VISIBLE",
            rmux_emu::screen::ScreenMode::CURSOR_VERY_VISIBLE.bits() as i64,
        ),
        (
            "MODE_CURSOR_BLINKING_SET",
            rmux_emu::screen::ScreenMode::CURSOR_BLINKING_SET.bits() as i64,
        ),
        (
            "MODE_KEYS_EXTENDED_2",
            rmux_emu::screen::ScreenMode::KEYS_EXTENDED_2.bits() as i64,
        ),
        (
            "MODE_THEME_UPDATES",
            rmux_emu::screen::ScreenMode::THEME_UPDATES.bits() as i64,
        ),
        (
            "MODE_SYNC",
            rmux_emu::screen::ScreenMode::SYNC.bits() as i64,
        ),
        (
            "ALL_MODES",
            rmux_emu::screen::ScreenMode::ALL_MODES.bits() as i64,
        ),
        (
            "ALL_MOUSE_MODES",
            rmux_emu::screen::ScreenMode::ALL_MOUSE_MODES.bits() as i64,
        ),
        (
            "MOTION_MOUSE_MODES",
            rmux_emu::screen::ScreenMode::MOTION_MOUSE_MODES.bits() as i64,
        ),
        (
            "CURSOR_MODES",
            rmux_emu::screen::ScreenMode::CURSOR_MODES.bits() as i64,
        ),
        (
            "EXTENDED_KEY_MODES",
            rmux_emu::screen::ScreenMode::EXTENDED_KEY_MODES.bits() as i64,
        ),
        (
            "COLOUR_FLAG_256",
            rmux_emu::colour::ColourFlags::_256.bits() as i64,
        ),
        (
            "COLOUR_FLAG_RGB",
            rmux_emu::colour::ColourFlags::RGB.bits() as i64,
        ),
        (
            "COLOUR_FLAG_THEME",
            rmux_emu::colour::ColourFlags::THEME.bits() as i64,
        ),
        (
            "GRID_ATTR_BRIGHT",
            rmux_emu::cell::GridAttributes::BRIGHT.bits() as i64,
        ),
        (
            "GRID_ATTR_DIM",
            rmux_emu::cell::GridAttributes::DIM.bits() as i64,
        ),
        (
            "GRID_ATTR_UNDERSCORE",
            rmux_emu::cell::GridAttributes::UNDERSCORE.bits() as i64,
        ),
        (
            "GRID_ATTR_BLINK",
            rmux_emu::cell::GridAttributes::BLINK.bits() as i64,
        ),
        (
            "GRID_ATTR_REVERSE",
            rmux_emu::cell::GridAttributes::REVERSE.bits() as i64,
        ),
        (
            "GRID_ATTR_HIDDEN",
            rmux_emu::cell::GridAttributes::HIDDEN.bits() as i64,
        ),
        (
            "GRID_ATTR_ITALICS",
            rmux_emu::cell::GridAttributes::ITALICS.bits() as i64,
        ),
        (
            "GRID_ATTR_CHARSET",
            rmux_emu::cell::GridAttributes::CHARSET.bits() as i64,
        ),
        (
            "GRID_ATTR_STRIKETHROUGH",
            rmux_emu::cell::GridAttributes::STRIKETHROUGH.bits() as i64,
        ),
        (
            "GRID_ATTR_UNDERSCORE_2",
            rmux_emu::cell::GridAttributes::UNDERSCORE_2.bits() as i64,
        ),
        (
            "GRID_ATTR_UNDERSCORE_3",
            rmux_emu::cell::GridAttributes::UNDERSCORE_3.bits() as i64,
        ),
        (
            "GRID_ATTR_UNDERSCORE_4",
            rmux_emu::cell::GridAttributes::UNDERSCORE_4.bits() as i64,
        ),
        (
            "GRID_ATTR_UNDERSCORE_5",
            rmux_emu::cell::GridAttributes::UNDERSCORE_5.bits() as i64,
        ),
        (
            "GRID_ATTR_OVERLINE",
            rmux_emu::cell::GridAttributes::OVERLINE.bits() as i64,
        ),
        (
            "GRID_ATTR_NOATTR",
            rmux_emu::cell::GridAttributes::NOATTR.bits() as i64,
        ),
        (
            "GRID_ATTR_ALL_UNDERSCORE",
            rmux_emu::cell::GridAttributes::ALL_UNDERSCORE.bits() as i64,
        ),
        (
            "GRID_FLAG_FG256",
            rmux_emu::cell::GridCellFlags::FG256.bits() as i64,
        ),
        (
            "GRID_FLAG_BG256",
            rmux_emu::cell::GridCellFlags::BG256.bits() as i64,
        ),
        (
            "GRID_FLAG_PADDING",
            rmux_emu::cell::GridCellFlags::PADDING.bits() as i64,
        ),
        (
            "GRID_FLAG_EXTENDED",
            rmux_emu::cell::GridCellFlags::EXTENDED.bits() as i64,
        ),
        (
            "GRID_FLAG_SELECTED",
            rmux_emu::cell::GridCellFlags::SELECTED.bits() as i64,
        ),
        (
            "GRID_FLAG_NOPALETTE",
            rmux_emu::cell::GridCellFlags::NOPALETTE.bits() as i64,
        ),
        (
            "GRID_FLAG_CLEARED",
            rmux_emu::cell::GridCellFlags::CLEARED.bits() as i64,
        ),
        (
            "GRID_FLAG_TAB",
            rmux_emu::cell::GridCellFlags::TAB.bits() as i64,
        ),
        (
            "GRID_LINE_WRAPPED",
            rmux_emu::grid::GridLineFlags::WRAPPED.bits() as i64,
        ),
        (
            "GRID_LINE_EXTENDED",
            rmux_emu::grid::GridLineFlags::EXTENDED.bits() as i64,
        ),
        (
            "GRID_LINE_DEAD",
            rmux_emu::grid::GridLineFlags::DEAD.bits() as i64,
        ),
        (
            "GRID_LINE_START_PROMPT",
            rmux_emu::grid::GridLineFlags::START_PROMPT.bits() as i64,
        ),
        (
            "GRID_LINE_SECOND_PROMPT",
            rmux_emu::grid::GridLineFlags::SECOND_PROMPT.bits() as i64,
        ),
        (
            "GRID_LINE_START_COMMAND",
            rmux_emu::grid::GridLineFlags::START_COMMAND.bits() as i64,
        ),
        (
            "GRID_LINE_START_OUTPUT",
            rmux_emu::grid::GridLineFlags::START_OUTPUT.bits() as i64,
        ),
        (
            "GRID_LINE_END_OUTPUT",
            rmux_emu::grid::GridLineFlags::END_OUTPUT.bits() as i64,
        ),
        (
            "GRID_LINE_HYPERLINK",
            rmux_emu::grid::GridLineFlags::HYPERLINK.bits() as i64,
        ),
        (
            "GRID_LINE_OSC133_FLAGS",
            rmux_emu::grid::GridLineFlags::OSC133_FLAGS.bits() as i64,
        ),
        (
            "GRID_STRING_WITH_SEQUENCES",
            rmux_emu::grid::GridStringFlags::WITH_SEQUENCES.bits() as i64,
        ),
        (
            "GRID_STRING_ESCAPE_SEQUENCES",
            rmux_emu::grid::GridStringFlags::ESCAPE_SEQUENCES.bits() as i64,
        ),
        (
            "GRID_STRING_TRIM_SPACES",
            rmux_emu::grid::GridStringFlags::TRIM_SPACES.bits() as i64,
        ),
        (
            "GRID_STRING_USED_ONLY",
            rmux_emu::grid::GridStringFlags::USED_ONLY.bits() as i64,
        ),
        (
            "GRID_STRING_EMPTY_CELLS",
            rmux_emu::grid::GridStringFlags::EMPTY_CELLS.bits() as i64,
        ),
        (
            "GRID_HISTORY",
            rmux_emu::grid::GridFlags::HISTORY.bits() as i64,
        ),
        (
            "SCREEN_WRITE_SYNC",
            rmux_emu::screen::write::ScreenWriteFlags::SYNC.bits() as i64,
        ),
        (
            "SCREEN_WRITE_OBSCURED",
            rmux_emu::screen::write::ScreenWriteFlags::OBSCURED.bits() as i64,
        ),
        (
            "SCREEN_WRITE_CHECKED_IF_OBSCURED",
            rmux_emu::screen::write::ScreenWriteFlags::CHECKED_IF_OBSCURED.bits() as i64,
        ),
        (
            "WINDOW_MODE_HIDE_PANE_STATUS",
            rmux_server::modes::WindowModeFlags::HIDE_PANE_STATUS.bits() as i64,
        ),
        (
            "WINDOW_MODE_NO_STACK",
            rmux_server::modes::WindowModeFlags::NO_STACK.bits() as i64,
        ),
        (
            "WINDOW_MODE_FILL_WINDOW",
            rmux_server::modes::WindowModeFlags::FILL_WINDOW.bits() as i64,
        ),
        (
            "WINDOW_MODE_HIDE_SCROLLBARS",
            rmux_server::modes::WindowModeFlags::HIDE_SCROLLBARS.bits() as i64,
        ),
        (
            "PANE_REDRAW",
            rmux_server::model::PaneFlags::REDRAW.bits() as i64,
        ),
        (
            "PANE_DROP",
            rmux_server::model::PaneFlags::DROP.bits() as i64,
        ),
        (
            "PANE_FOCUSED",
            rmux_server::model::PaneFlags::FOCUSED.bits() as i64,
        ),
        (
            "PANE_VISITED",
            rmux_server::model::PaneFlags::VISITED.bits() as i64,
        ),
        (
            "PANE_ZOOMED",
            rmux_server::model::PaneFlags::ZOOMED.bits() as i64,
        ),
        (
            "PANE_NEWSTATUS",
            rmux_server::model::PaneFlags::NEWSTATUS.bits() as i64,
        ),
        (
            "PANE_INPUTOFF",
            rmux_server::model::PaneFlags::INPUTOFF.bits() as i64,
        ),
        (
            "PANE_CHANGED",
            rmux_server::model::PaneFlags::CHANGED.bits() as i64,
        ),
        (
            "PANE_EXITED",
            rmux_server::model::PaneFlags::EXITED.bits() as i64,
        ),
        (
            "PANE_STATUSREADY",
            rmux_server::model::PaneFlags::STATUSREADY.bits() as i64,
        ),
        (
            "PANE_STATUSDRAWN",
            rmux_server::model::PaneFlags::STATUSDRAWN.bits() as i64,
        ),
        (
            "PANE_EMPTY",
            rmux_server::model::PaneFlags::EMPTY.bits() as i64,
        ),
        (
            "PANE_STYLECHANGED",
            rmux_server::model::PaneFlags::STYLECHANGED.bits() as i64,
        ),
        (
            "PANE_THEMECHANGED",
            rmux_server::model::PaneFlags::THEMECHANGED.bits() as i64,
        ),
        (
            "PANE_UNSEENCHANGES",
            rmux_server::model::PaneFlags::UNSEENCHANGES.bits() as i64,
        ),
        (
            "PANE_REDRAWSCROLLBAR",
            rmux_server::model::PaneFlags::REDRAWSCROLLBAR.bits() as i64,
        ),
        (
            "PANE_DESTROYED",
            rmux_server::model::PaneFlags::DESTROYED.bits() as i64,
        ),
        (
            "PANE_CMDRUNNING",
            rmux_server::model::PaneFlags::CMDRUNNING.bits() as i64,
        ),
        (
            "PANE_ACTIVITY",
            rmux_server::model::PaneFlags::ACTIVITY.bits() as i64,
        ),
        (
            "PANE_CLOSEONCLICK",
            rmux_server::model::PaneFlags::CLOSEONCLICK.bits() as i64,
        ),
        (
            "PANE_CAPTUREALLKEYS",
            rmux_server::model::PaneFlags::CAPTUREALLKEYS.bits() as i64,
        ),
        (
            "PANE_FLOATOVERZOOM",
            rmux_server::model::PaneFlags::FLOATOVERZOOM.bits() as i64,
        ),
        (
            "PANE_CLOSEONCANCEL",
            rmux_server::model::PaneFlags::CLOSEONCANCEL.bits() as i64,
        ),
        (
            "WINDOW_BELL",
            rmux_server::model::WindowFlags::BELL.bits() as i64,
        ),
        (
            "WINDOW_ACTIVITY",
            rmux_server::model::WindowFlags::ACTIVITY.bits() as i64,
        ),
        (
            "WINDOW_SILENCE",
            rmux_server::model::WindowFlags::SILENCE.bits() as i64,
        ),
        (
            "WINDOW_ZOOMED",
            rmux_server::model::WindowFlags::ZOOMED.bits() as i64,
        ),
        (
            "WINDOW_WASZOOMED",
            rmux_server::model::WindowFlags::WASZOOMED.bits() as i64,
        ),
        (
            "WINDOW_RESIZE",
            rmux_server::model::WindowFlags::RESIZE.bits() as i64,
        ),
        (
            "WINDOW_ALERTFLAGS",
            rmux_server::model::WindowFlags::ALERTFLAGS.bits() as i64,
        ),
        (
            "WINLINK_BELL",
            rmux_server::model::WinlinkFlags::BELL.bits() as i64,
        ),
        (
            "WINLINK_ACTIVITY",
            rmux_server::model::WinlinkFlags::ACTIVITY.bits() as i64,
        ),
        (
            "WINLINK_SILENCE",
            rmux_server::model::WinlinkFlags::SILENCE.bits() as i64,
        ),
        (
            "WINLINK_ALERTFLAGS",
            rmux_server::model::WinlinkFlags::ALERTFLAGS.bits() as i64,
        ),
        (
            "WINLINK_VISITED",
            rmux_server::model::WinlinkFlags::VISITED.bits() as i64,
        ),
        (
            "LAYOUT_CELL_FLOATING",
            rmux_server::layout::LayoutCellFlags::FLOATING.bits() as i64,
        ),
        (
            "ENVIRON_HIDDEN",
            rmux_server::options::environment::EnvironmentFlags::HIDDEN.bits() as i64,
        ),
        (
            "SESSION_ALERTED",
            rmux_server::model::SessionFlags::ALERTED.bits() as i64,
        ),
        (
            "MOUSE_MASK_BUTTONS",
            rmux_util::key::MouseButtonBits::BUTTONS.bits() as i64,
        ),
        (
            "MOUSE_MASK_SHIFT",
            rmux_util::key::MouseButtonBits::SHIFT.bits() as i64,
        ),
        (
            "MOUSE_MASK_META",
            rmux_util::key::MouseButtonBits::META.bits() as i64,
        ),
        (
            "MOUSE_MASK_CTRL",
            rmux_util::key::MouseButtonBits::CTRL.bits() as i64,
        ),
        (
            "MOUSE_MASK_DRAG",
            rmux_util::key::MouseButtonBits::DRAG.bits() as i64,
        ),
        (
            "MOUSE_MASK_MODIFIERS",
            rmux_util::key::MouseButtonBits::MODIFIERS.bits() as i64,
        ),
        (
            "TERM_256COLOURS",
            rmux_tty::term::TtyTermFlags::_256COLOURS.bits() as i64,
        ),
        (
            "TERM_NOAM",
            rmux_tty::term::TtyTermFlags::NOAM.bits() as i64,
        ),
        (
            "TERM_DECSLRM",
            rmux_tty::term::TtyTermFlags::DECSLRM.bits() as i64,
        ),
        (
            "TERM_DECFRA",
            rmux_tty::term::TtyTermFlags::DECFRA.bits() as i64,
        ),
        (
            "TERM_RGBCOLOURS",
            rmux_tty::term::TtyTermFlags::RGBCOLOURS.bits() as i64,
        ),
        (
            "TERM_VT100LIKE",
            rmux_tty::term::TtyTermFlags::VT100LIKE.bits() as i64,
        ),
        (
            "TERM_SIXEL",
            rmux_tty::term::TtyTermFlags::SIXEL.bits() as i64,
        ),
        (
            "TERM_INVALIDMS",
            rmux_tty::term::TtyTermFlags::INVALIDMS.bits() as i64,
        ),
        (
            "TERM_NOREPLACE",
            rmux_tty::term::TtyTermFlags::NOREPLACE.bits() as i64,
        ),
        (
            "TTY_NOCURSOR",
            rmux_tty::tty::TtyFlags::NOCURSOR.bits() as i64,
        ),
        ("TTY_FREEZE", rmux_tty::tty::TtyFlags::FREEZE.bits() as i64),
        ("TTY_TIMER", rmux_tty::tty::TtyFlags::TIMER.bits() as i64),
        (
            "TTY_NOBLOCK",
            rmux_tty::tty::TtyFlags::NOBLOCK.bits() as i64,
        ),
        (
            "TTY_STARTED",
            rmux_tty::tty::TtyFlags::STARTED.bits() as i64,
        ),
        ("TTY_OPENED", rmux_tty::tty::TtyFlags::OPENED.bits() as i64),
        (
            "TTY_OSC52QUERY",
            rmux_tty::tty::TtyFlags::OSC52QUERY.bits() as i64,
        ),
        ("TTY_BLOCK", rmux_tty::tty::TtyFlags::BLOCK.bits() as i64),
        ("TTY_HAVEDA", rmux_tty::tty::TtyFlags::HAVEDA.bits() as i64),
        (
            "TTY_HAVEXDA",
            rmux_tty::tty::TtyFlags::HAVEXDA.bits() as i64,
        ),
        (
            "TTY_SYNCING",
            rmux_tty::tty::TtyFlags::SYNCING.bits() as i64,
        ),
        (
            "TTY_HAVEDA2",
            rmux_tty::tty::TtyFlags::HAVEDA2.bits() as i64,
        ),
        (
            "TTY_WINSIZEQUERY",
            rmux_tty::tty::TtyFlags::WINSIZEQUERY.bits() as i64,
        ),
        ("TTY_WAITFG", rmux_tty::tty::TtyFlags::WAITFG.bits() as i64),
        ("TTY_WAITBG", rmux_tty::tty::TtyFlags::WAITBG.bits() as i64),
        (
            "TTY_BRACKETPASTE",
            rmux_tty::tty::TtyFlags::BRACKETPASTE.bits() as i64,
        ),
        (
            "TTY_HAVESYNC",
            rmux_tty::tty::TtyFlags::HAVESYNC.bits() as i64,
        ),
        (
            "TTY_ALL_REQUEST_FLAGS",
            rmux_tty::tty::TtyFlags::ALL_REQUEST_FLAGS.bits() as i64,
        ),
        (
            "TTY_CTX_WRAPPED",
            rmux_tty::draw::TtyCtxFlags::WRAPPED.bits() as i64,
        ),
        (
            "TTY_CTX_INVISIBLE_PANES",
            rmux_tty::draw::TtyCtxFlags::INVISIBLE_PANES.bits() as i64,
        ),
        (
            "TTY_CTX_WINDOW_BIGGER",
            rmux_tty::draw::TtyCtxFlags::WINDOW_BIGGER.bits() as i64,
        ),
        (
            "TTY_CTX_SYNC",
            rmux_tty::draw::TtyCtxFlags::SYNC.bits() as i64,
        ),
        (
            "TTY_CTX_CELL_INVALIDATE",
            rmux_tty::draw::TtyCtxFlags::CELL_INVALIDATE.bits() as i64,
        ),
        (
            "TTY_CTX_PANE_OBSCURED",
            rmux_tty::draw::TtyCtxFlags::PANE_OBSCURED.bits() as i64,
        ),
        (
            "CMD_FIND_PREFER_UNATTACHED",
            rmux_server::cmd::find::CmdFindFlags::PREFER_UNATTACHED.bits() as i64,
        ),
        (
            "CMD_FIND_QUIET",
            rmux_server::cmd::find::CmdFindFlags::QUIET.bits() as i64,
        ),
        (
            "CMD_FIND_WINDOW_INDEX",
            rmux_server::cmd::find::CmdFindFlags::WINDOW_INDEX.bits() as i64,
        ),
        (
            "CMD_FIND_DEFAULT_MARKED",
            rmux_server::cmd::find::CmdFindFlags::DEFAULT_MARKED.bits() as i64,
        ),
        (
            "CMD_FIND_EXACT_SESSION",
            rmux_server::cmd::find::CmdFindFlags::EXACT_SESSION.bits() as i64,
        ),
        (
            "CMD_FIND_EXACT_WINDOW",
            rmux_server::cmd::find::CmdFindFlags::EXACT_WINDOW.bits() as i64,
        ),
        (
            "CMD_FIND_CANFAIL",
            rmux_server::cmd::find::CmdFindFlags::CANFAIL.bits() as i64,
        ),
        (
            "CMD_PARSE_QUIET",
            rmux_server::cmd::parse::CmdParseFlags::QUIET.bits() as i64,
        ),
        (
            "CMD_PARSE_PARSEONLY",
            rmux_server::cmd::parse::CmdParseFlags::PARSEONLY.bits() as i64,
        ),
        (
            "CMD_PARSE_NOALIAS",
            rmux_server::cmd::parse::CmdParseFlags::NOALIAS.bits() as i64,
        ),
        (
            "CMD_PARSE_VERBOSE",
            rmux_server::cmd::parse::CmdParseFlags::VERBOSE.bits() as i64,
        ),
        (
            "CMD_PARSE_ONEGROUP",
            rmux_server::cmd::parse::CmdParseFlags::ONEGROUP.bits() as i64,
        ),
        (
            "CMDQ_STATE_REPEAT",
            rmux_server::cmd::queue::QueueStateFlags::REPEAT.bits() as i64,
        ),
        (
            "CMDQ_STATE_CONTROL",
            rmux_server::cmd::queue::QueueStateFlags::CONTROL.bits() as i64,
        ),
        (
            "CMDQ_STATE_NOHOOKS",
            rmux_server::cmd::queue::QueueStateFlags::NOHOOKS.bits() as i64,
        ),
        (
            "CMD_STARTSERVER",
            rmux_server::cmd::CommandFlags::STARTSERVER.bits() as i64,
        ),
        (
            "CMD_READONLY",
            rmux_server::cmd::CommandFlags::READONLY.bits() as i64,
        ),
        (
            "CMD_AFTERHOOK",
            rmux_server::cmd::CommandFlags::AFTERHOOK.bits() as i64,
        ),
        (
            "CMD_CLIENT_CFLAG",
            rmux_server::cmd::CommandFlags::CLIENT_CFLAG.bits() as i64,
        ),
        (
            "CMD_CLIENT_TFLAG",
            rmux_server::cmd::CommandFlags::CLIENT_TFLAG.bits() as i64,
        ),
        (
            "CMD_CLIENT_CANFAIL",
            rmux_server::cmd::CommandFlags::CLIENT_CANFAIL.bits() as i64,
        ),
        (
            "PROMPT_SINGLE",
            rmux_server::ui::prompt::PromptFlags::SINGLE.bits() as i64,
        ),
        (
            "PROMPT_NUMERIC",
            rmux_server::ui::prompt::PromptFlags::NUMERIC.bits() as i64,
        ),
        (
            "PROMPT_INCREMENTAL",
            rmux_server::ui::prompt::PromptFlags::INCREMENTAL.bits() as i64,
        ),
        (
            "PROMPT_NOFORMAT",
            rmux_server::ui::prompt::PromptFlags::NOFORMAT.bits() as i64,
        ),
        (
            "PROMPT_KEY",
            rmux_server::ui::prompt::PromptFlags::KEY.bits() as i64,
        ),
        (
            "PROMPT_ACCEPT",
            rmux_server::ui::prompt::PromptFlags::ACCEPT.bits() as i64,
        ),
        (
            "PROMPT_QUOTENEXT",
            rmux_server::ui::prompt::PromptFlags::QUOTENEXT.bits() as i64,
        ),
        (
            "PROMPT_BSPACE_EXIT",
            rmux_server::ui::prompt::PromptFlags::BSPACE_EXIT.bits() as i64,
        ),
        (
            "PROMPT_NOFREEZE",
            rmux_server::ui::prompt::PromptFlags::NOFREEZE.bits() as i64,
        ),
        (
            "PROMPT_COMMANDMODE",
            rmux_server::ui::prompt::PromptFlags::COMMANDMODE.bits() as i64,
        ),
        (
            "PROMPT_ISPANE",
            rmux_server::ui::prompt::PromptFlags::ISPANE.bits() as i64,
        ),
        (
            "PROMPT_ISMODE",
            rmux_server::ui::prompt::PromptFlags::ISMODE.bits() as i64,
        ),
        (
            "PROMPT_EDITARROWS",
            rmux_server::ui::prompt::PromptFlags::EDITARROWS.bits() as i64,
        ),
        (
            "CLIENT_TERMINAL",
            rmux_server::client::ClientFlags::TERMINAL.bits() as i64,
        ),
        (
            "CLIENT_LOGIN",
            rmux_server::client::ClientFlags::LOGIN.bits() as i64,
        ),
        (
            "CLIENT_EXIT",
            rmux_server::client::ClientFlags::EXIT.bits() as i64,
        ),
        (
            "CLIENT_REDRAWWINDOW",
            rmux_server::client::ClientFlags::REDRAWWINDOW.bits() as i64,
        ),
        (
            "CLIENT_REDRAWSTATUS",
            rmux_server::client::ClientFlags::REDRAWSTATUS.bits() as i64,
        ),
        (
            "CLIENT_REPEAT",
            rmux_server::client::ClientFlags::REPEAT.bits() as i64,
        ),
        (
            "CLIENT_SUSPENDED",
            rmux_server::client::ClientFlags::SUSPENDED.bits() as i64,
        ),
        (
            "CLIENT_ATTACHED",
            rmux_server::client::ClientFlags::ATTACHED.bits() as i64,
        ),
        (
            "CLIENT_EXITED",
            rmux_server::client::ClientFlags::EXITED.bits() as i64,
        ),
        (
            "CLIENT_DEAD",
            rmux_server::client::ClientFlags::DEAD.bits() as i64,
        ),
        (
            "CLIENT_REDRAWBORDERS",
            rmux_server::client::ClientFlags::REDRAWBORDERS.bits() as i64,
        ),
        (
            "CLIENT_READONLY",
            rmux_server::client::ClientFlags::READONLY.bits() as i64,
        ),
        (
            "CLIENT_NOSTARTSERVER",
            rmux_server::client::ClientFlags::NOSTARTSERVER.bits() as i64,
        ),
        (
            "CLIENT_CONTROL",
            rmux_server::client::ClientFlags::CONTROL.bits() as i64,
        ),
        (
            "CLIENT_CONTROLCONTROL",
            rmux_server::client::ClientFlags::CONTROLCONTROL.bits() as i64,
        ),
        (
            "CLIENT_FOCUSED",
            rmux_server::client::ClientFlags::FOCUSED.bits() as i64,
        ),
        (
            "CLIENT_UTF8",
            rmux_server::client::ClientFlags::UTF8.bits() as i64,
        ),
        (
            "CLIENT_IGNORESIZE",
            rmux_server::client::ClientFlags::IGNORESIZE.bits() as i64,
        ),
        (
            "CLIENT_IDENTIFIED",
            rmux_server::client::ClientFlags::IDENTIFIED.bits() as i64,
        ),
        (
            "CLIENT_STATUSFORCE",
            rmux_server::client::ClientFlags::STATUSFORCE.bits() as i64,
        ),
        (
            "CLIENT_DOUBLECLICK",
            rmux_server::client::ClientFlags::DOUBLECLICK.bits() as i64,
        ),
        (
            "CLIENT_TRIPLECLICK",
            rmux_server::client::ClientFlags::TRIPLECLICK.bits() as i64,
        ),
        (
            "CLIENT_SIZECHANGED",
            rmux_server::client::ClientFlags::SIZECHANGED.bits() as i64,
        ),
        (
            "CLIENT_STATUSOFF",
            rmux_server::client::ClientFlags::STATUSOFF.bits() as i64,
        ),
        (
            "CLIENT_REDRAWSTATUSALWAYS",
            rmux_server::client::ClientFlags::REDRAWSTATUSALWAYS.bits() as i64,
        ),
        (
            "CLIENT_CONTROL_NOOUTPUT",
            rmux_server::client::ClientFlags::CONTROL_NOOUTPUT.bits() as i64,
        ),
        (
            "CLIENT_DEFAULTSOCKET",
            rmux_server::client::ClientFlags::DEFAULTSOCKET.bits() as i64,
        ),
        (
            "CLIENT_STARTSERVER",
            rmux_server::client::ClientFlags::STARTSERVER.bits() as i64,
        ),
        (
            "CLIENT_REDRAWMENU",
            rmux_server::client::ClientFlags::REDRAWMENU.bits() as i64,
        ),
        (
            "CLIENT_NOFORK",
            rmux_server::client::ClientFlags::NOFORK.bits() as i64,
        ),
        (
            "CLIENT_REDRAWSCROLLBARS",
            rmux_server::client::ClientFlags::REDRAWSCROLLBARS.bits() as i64,
        ),
        (
            "CLIENT_CONTROL_PAUSEAFTER",
            rmux_server::client::ClientFlags::CONTROL_PAUSEAFTER.bits() as i64,
        ),
        (
            "CLIENT_CONTROL_WAITEXIT",
            rmux_server::client::ClientFlags::CONTROL_WAITEXIT.bits() as i64,
        ),
        (
            "CLIENT_WINDOWSIZECHANGED",
            rmux_server::client::ClientFlags::WINDOWSIZECHANGED.bits() as i64,
        ),
        (
            "CLIENT_CONTROL_NEWLAYOUTS",
            rmux_server::client::ClientFlags::CONTROL_NEWLAYOUTS.bits() as i64,
        ),
        (
            "CLIENT_BRACKETPASTING",
            rmux_server::client::ClientFlags::BRACKETPASTING.bits() as i64,
        ),
        (
            "CLIENT_ASSUMEPASTING",
            rmux_server::client::ClientFlags::ASSUMEPASTING.bits() as i64,
        ),
        (
            "CLIENT_WRITE_ACK",
            rmux_server::client::ClientFlags::WRITE_ACK.bits() as i64,
        ),
        (
            "CLIENT_NO_DETACH_ON_DESTROY",
            rmux_server::client::ClientFlags::NO_DETACH_ON_DESTROY.bits() as i64,
        ),
        (
            "CLIENT_CONTROL_DISCARD",
            rmux_server::client::ClientFlags::CONTROL_DISCARD.bits() as i64,
        ),
        (
            "CLIENT_ALLREDRAWFLAGS",
            rmux_server::client::ClientFlags::ALLREDRAWFLAGS.bits() as i64,
        ),
        (
            "CLIENT_UNATTACHEDFLAGS",
            rmux_server::client::ClientFlags::UNATTACHEDFLAGS.bits() as i64,
        ),
        (
            "CLIENT_NODETACHFLAGS",
            rmux_server::client::ClientFlags::NODETACHFLAGS.bits() as i64,
        ),
        (
            "CLIENT_NOSIZEFLAGS",
            rmux_server::client::ClientFlags::NOSIZEFLAGS.bits() as i64,
        ),
        (
            "MONITOR_NOTIFY_INITIAL",
            rmux_server::model::monitor::MonitorFlags::INITIAL.bits() as i64,
        ),
        (
            "MONITOR_NOTIFY_TRUE",
            rmux_server::model::monitor::MonitorFlags::TRUE.bits() as i64,
        ),
        (
            "KEY_BINDING_REPEAT",
            rmux_server::cmd::key_bindings::KeyBindingFlags::REPEAT.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_NONE",
            rmux_server::options::OptionsScope::NONE.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_SERVER",
            rmux_server::options::OptionsScope::SERVER.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_SESSION",
            rmux_server::options::OptionsScope::SESSION.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_WINDOW",
            rmux_server::options::OptionsScope::WINDOW.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_PANE",
            rmux_server::options::OptionsScope::PANE.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_IS_ARRAY",
            rmux_server::options::OptionsTableFlags::ARRAY.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_IS_HOOK",
            rmux_server::options::OptionsTableFlags::HOOK.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_IS_STYLE",
            rmux_server::options::OptionsTableFlags::STYLE.bits() as i64,
        ),
        (
            "OPTIONS_TABLE_IS_COLOUR",
            rmux_server::options::OptionsTableFlags::COLOUR.bits() as i64,
        ),
        (
            "SPAWN_KILL",
            rmux_server::model::spawn::SpawnFlags::KILL.bits() as i64,
        ),
        (
            "SPAWN_DETACHED",
            rmux_server::model::spawn::SpawnFlags::DETACHED.bits() as i64,
        ),
        (
            "SPAWN_RESPAWN",
            rmux_server::model::spawn::SpawnFlags::RESPAWN.bits() as i64,
        ),
        (
            "SPAWN_BEFORE",
            rmux_server::model::spawn::SpawnFlags::BEFORE.bits() as i64,
        ),
        (
            "SPAWN_NONOTIFY",
            rmux_server::model::spawn::SpawnFlags::NONOTIFY.bits() as i64,
        ),
        (
            "SPAWN_FULLSIZE",
            rmux_server::model::spawn::SpawnFlags::FULLSIZE.bits() as i64,
        ),
        (
            "SPAWN_EMPTY",
            rmux_server::model::spawn::SpawnFlags::EMPTY.bits() as i64,
        ),
        (
            "SPAWN_ZOOM",
            rmux_server::model::spawn::SpawnFlags::ZOOM.bits() as i64,
        ),
        (
            "SPAWN_FLOATING",
            rmux_server::model::spawn::SpawnFlags::FLOATING.bits() as i64,
        ),
        (
            "SPAWN_HORIZONTAL",
            rmux_server::model::spawn::SpawnFlags::HORIZONTAL.bits() as i64,
        ),
        (
            "SPAWN_SPLIT",
            rmux_server::model::spawn::SpawnFlags::SPLIT.bits() as i64,
        ),
        (
            "SPAWN_MODAL",
            rmux_server::model::spawn::SpawnFlags::MODAL.bits() as i64,
        ),
        (
            "SPAWN_FLOATOVERZOOM",
            rmux_server::model::spawn::SpawnFlags::FLOATOVERZOOM.bits() as i64,
        ),
        (
            "FORMAT_STATUS",
            rmux_server::format::FormatFlags::STATUS.bits() as i64,
        ),
        (
            "FORMAT_FORCE",
            rmux_server::format::FormatFlags::FORCE.bits() as i64,
        ),
        (
            "FORMAT_NOJOBS",
            rmux_server::format::FormatFlags::NOJOBS.bits() as i64,
        ),
        (
            "FORMAT_VERBOSE",
            rmux_server::format::FormatFlags::VERBOSE.bits() as i64,
        ),
        (
            "FORMAT_LAST",
            rmux_server::format::FormatFlags::LAST.bits() as i64,
        ),
        (
            "FORMAT_NONE",
            rmux_server::format::FormatFlags::NONE.bits() as i64,
        ),
        (
            "FORMAT_PANE",
            rmux_server::format::FormatTagFlags::PANE.bits() as i64,
        ),
        (
            "FORMAT_WINDOW",
            rmux_server::format::FormatTagFlags::WINDOW.bits() as i64,
        ),
        (
            "JOB_NOWAIT",
            rmux_server::server::job::JobFlags::NOWAIT.bits() as i64,
        ),
        (
            "JOB_KEEPWRITE",
            rmux_server::server::job::JobFlags::KEEPWRITE.bits() as i64,
        ),
        (
            "JOB_PTY",
            rmux_server::server::job::JobFlags::PTY.bits() as i64,
        ),
        (
            "JOB_DEFAULTSHELL",
            rmux_server::server::job::JobFlags::DEFAULTSHELL.bits() as i64,
        ),
        (
            "JOB_SHOWSTDERR",
            rmux_server::server::job::JobFlags::SHOWSTDERR.bits() as i64,
        ),
        (
            "CMD_LIST_PRINT_ESCAPED",
            rmux_server::cmd::CommandListPrintFlags::ESCAPED.bits() as i64,
        ),
        (
            "CMD_LIST_PRINT_NO_GROUPS",
            rmux_server::cmd::CommandListPrintFlags::NO_GROUPS.bits() as i64,
        ),
        (
            "LAYOUT_CUSTOM_OLD_FORMAT",
            rmux_server::layout::LayoutDumpFlags::FORMAT.bits() as i64,
        ),
        (
            "MENU_NOMOUSE",
            rmux_server::ui::menu::MenuFlags::NOMOUSE.bits() as i64,
        ),
        (
            "MENU_TAB",
            rmux_server::ui::menu::MenuFlags::TAB.bits() as i64,
        ),
        (
            "MENU_STAYOPEN",
            rmux_server::ui::menu::MenuFlags::STAYOPEN.bits() as i64,
        ),
        (
            "SERVER_ACL_READONLY",
            rmux_server::server::acl::ServerAclFlags::READONLY.bits() as i64,
        ),
        (
            "SERVER_ACL_IS_GROUP",
            rmux_server::server::acl::ServerAclFlags::IS_GROUP.bits() as i64,
        ),
        (
            "ALERT_NONE",
            rmux_server::model::alerts::AlertPolicy::None as i64,
        ),
        (
            "ALERT_ANY",
            rmux_server::model::alerts::AlertPolicy::Any as i64,
        ),
        (
            "ALERT_CURRENT",
            rmux_server::model::alerts::AlertPolicy::Current as i64,
        ),
        (
            "ALERT_OTHER",
            rmux_server::model::alerts::AlertPolicy::Other as i64,
        ),
        (
            "VISUAL_OFF",
            rmux_server::model::alerts::VisualPolicy::Off as i64,
        ),
        (
            "VISUAL_ON",
            rmux_server::model::alerts::VisualPolicy::On as i64,
        ),
        (
            "VISUAL_BOTH",
            rmux_server::model::alerts::VisualPolicy::Both as i64,
        ),
        ("MODEKEY_EMACS", rmux_util::key::ModeKeys::Emacs as i64),
        ("MODEKEY_VI", rmux_util::key::ModeKeys::Vi as i64),
        ("CELL_INSIDE", rmux_emu::screen::BorderCell::Inside as i64),
        ("CELL_UD", rmux_emu::screen::BorderCell::Ud as i64),
        ("CELL_LR", rmux_emu::screen::BorderCell::Lr as i64),
        ("CELL_RD", rmux_emu::screen::BorderCell::Rd as i64),
        ("CELL_LD", rmux_emu::screen::BorderCell::Ld as i64),
        ("CELL_RU", rmux_emu::screen::BorderCell::Ru as i64),
        ("CELL_LU", rmux_emu::screen::BorderCell::Lu as i64),
        ("CELL_LRD", rmux_emu::screen::BorderCell::Lrd as i64),
        ("CELL_LRU", rmux_emu::screen::BorderCell::Lru as i64),
        ("CELL_URD", rmux_emu::screen::BorderCell::Urd as i64),
        ("CELL_ULD", rmux_emu::screen::BorderCell::Uld as i64),
        ("CELL_LRUD", rmux_emu::screen::BorderCell::Lrud as i64),
        ("CELL_NONE", rmux_emu::screen::BorderCell::None as i64),
        (
            "CELL_SCROLLBAR",
            rmux_emu::screen::BorderCell::Scrollbar as i64,
        ),
        (
            "PANE_BORDER_OFF",
            rmux_server::options::PaneBorderIndicator::Off as i64,
        ),
        (
            "PANE_BORDER_COLOUR",
            rmux_server::options::PaneBorderIndicator::Colour as i64,
        ),
        (
            "PANE_BORDER_ARROWS",
            rmux_server::options::PaneBorderIndicator::Arrows as i64,
        ),
        (
            "PANE_BORDER_BOTH",
            rmux_server::options::PaneBorderIndicator::Both as i64,
        ),
        (
            "WINDOW_PANE_NO_MODE",
            rmux_server::modes::PaneModeKind::NoMode as i64,
        ),
        (
            "WINDOW_PANE_COPY_MODE",
            rmux_server::modes::PaneModeKind::CopyMode as i64,
        ),
        (
            "WINDOW_PANE_VIEW_MODE",
            rmux_server::modes::PaneModeKind::ViewMode as i64,
        ),
        (
            "WINDOW_SIZE_LARGEST",
            rmux_server::model::WindowSizePolicy::Largest as i64,
        ),
        (
            "WINDOW_SIZE_SMALLEST",
            rmux_server::model::WindowSizePolicy::Smallest as i64,
        ),
        (
            "WINDOW_SIZE_MANUAL",
            rmux_server::model::WindowSizePolicy::Manual as i64,
        ),
        (
            "WINDOW_SIZE_LATEST",
            rmux_server::model::WindowSizePolicy::Latest as i64,
        ),
        (
            "PANE_STATUS_OFF",
            rmux_server::ui::status::PaneStatusPosition::Off as i64,
        ),
        (
            "PANE_STATUS_TOP",
            rmux_server::ui::status::PaneStatusPosition::Top as i64,
        ),
        (
            "PANE_STATUS_BOTTOM",
            rmux_server::ui::status::PaneStatusPosition::Bottom as i64,
        ),
        (
            "PANE_STATUS_TOP_FLOATING",
            rmux_server::ui::status::PaneStatusPosition::TopFloating as i64,
        ),
        (
            "PANE_STATUS_BOTTOM_FLOATING",
            rmux_server::ui::status::PaneStatusPosition::BottomFloating as i64,
        ),
        (
            "PANE_SCROLLBARS_OFF",
            rmux_server::ui::scrollbar::PaneScrollbarPolicy::Off as i64,
        ),
        (
            "PANE_SCROLLBARS_MODAL",
            rmux_server::ui::scrollbar::PaneScrollbarPolicy::Modal as i64,
        ),
        (
            "PANE_SCROLLBARS_ALWAYS",
            rmux_server::ui::scrollbar::PaneScrollbarPolicy::Always as i64,
        ),
        (
            "PANE_SCROLLBARS_AUTOHIDE",
            rmux_server::ui::scrollbar::PaneScrollbarPolicy::Autohide as i64,
        ),
        (
            "PANE_SCROLLBARS_RIGHT",
            rmux_server::ui::scrollbar::PaneScrollbarPosition::Right as i64,
        ),
        (
            "PANE_SCROLLBARS_LEFT",
            rmux_server::ui::scrollbar::PaneScrollbarPosition::Left as i64,
        ),
        (
            "MOUSE_WHEEL_UP",
            rmux_util::key::MouseButton::WheelUp as i64,
        ),
        (
            "MOUSE_WHEEL_DOWN",
            rmux_util::key::MouseButton::WheelDown as i64,
        ),
        (
            "MOUSE_BUTTON_1",
            rmux_util::key::MouseButton::Button1 as i64,
        ),
        (
            "MOUSE_BUTTON_2",
            rmux_util::key::MouseButton::Button2 as i64,
        ),
        (
            "MOUSE_BUTTON_3",
            rmux_util::key::MouseButton::Button3 as i64,
        ),
        (
            "MOUSE_BUTTON_6",
            rmux_util::key::MouseButton::Button6 as i64,
        ),
        (
            "MOUSE_BUTTON_7",
            rmux_util::key::MouseButton::Button7 as i64,
        ),
        (
            "MOUSE_BUTTON_8",
            rmux_util::key::MouseButton::Button8 as i64,
        ),
        (
            "MOUSE_BUTTON_9",
            rmux_util::key::MouseButton::Button9 as i64,
        ),
        (
            "MOUSE_BUTTON_10",
            rmux_util::key::MouseButton::Button10 as i64,
        ),
        (
            "MOUSE_BUTTON_11",
            rmux_util::key::MouseButton::Button11 as i64,
        ),
        (
            "COLOUR_THEME_MAGENTA",
            rmux_emu::colour::ColourTheme::Magenta as i64,
        ),
        (
            "STYLE_ALIGN_ABSOLUTE_CENTRE",
            rmux_emu::style::StyleAlign::AbsoluteCentre as i64,
        ),
        (
            "STYLE_RANGE_CONTROL",
            rmux_emu::style::StyleRangeType::Control as i64,
        ),
        (
            "STYLE_DEFAULT_SET",
            rmux_emu::style::StyleDefaultType::Set as i64,
        ),
        (
            "SCREEN_CURSOR_BAR",
            rmux_emu::screen::ScreenCursorStyle::Bar as i64,
        ),
        ("BOX_LINES_NONE", rmux_emu::screen::BoxLines::None as i64),
        (
            "PANE_LINES_ROUNDED",
            rmux_emu::screen::PaneLines::Rounded as i64,
        ),
        (
            "INPUT_REQUEST_QUEUE",
            rmux_emu::input::InputRequestType::Queue as i64,
        ),
        ("THEME_DARK", rmux_emu::colour::ClientTheme::Dark as i64),
        (
            "LAYOUT_WINDOWPANE",
            rmux_server::layout::LayoutType::Windowpane as i64,
        ),
        (
            "ARGS_COMMANDS",
            rmux_server::cmd::arguments::ArgsType::Commands as i64,
        ),
        (
            "ARGS_PARSE_COMMANDS",
            rmux_server::cmd::arguments::ArgsParseType::Commands as i64,
        ),
        (
            "CMD_RETURN_STOP",
            rmux_server::cmd::queue::CmdReturn::Stop as i64,
        ),
        (
            "CMD_PARSE_SUCCESS",
            rmux_server::cmd::parse::CmdParseStatus::Success as i64,
        ),
        (
            "PROMPT_CLOSE",
            rmux_server::ui::prompt::PromptResult::Close as i64,
        ),
        (
            "PROMPT_KEY_MOVE",
            rmux_server::ui::prompt::PromptKeyResult::Move as i64,
        ),
        (
            "MONITOR_ALL_WINDOWS",
            rmux_server::model::monitor::MonitorType::AllWindows as i64,
        ),
        (
            "EVENT_PAYLOAD_POINTER",
            rmux_server::server::events::EventPayloadType::Pointer as i64,
        ),
        (
            "OPTIONS_TABLE_COMMAND",
            rmux_server::options::OptionsTableType::Command as i64,
        ),
        (
            "CLIENT_EXIT_DETACH",
            rmux_server::client::ClientExitType::Detach as i64,
        ),
        ("C0_US", rmux_util::key::C0::US as i64),
        ("KEYC_NUSER", rmux_util::key::KeyCode::NUSER as i64),
        (
            "KEYC_CLICK_TIMEOUT",
            rmux_util::key::KeyCode::CLICK_TIMEOUT as i64,
        ),
        (
            "KEYC_MOUSE_LOCATION_SHIFT",
            rmux_util::key::KeyCode::MOUSE_LOCATION_SHIFT as i64,
        ),
        (
            "KEYC_MOUSE_BUTTON_SHIFT",
            rmux_util::key::KeyCode::MOUSE_BUTTON_SHIFT as i64,
        ),
        ("MOUSE_PARAM_MAX", rmux_util::key::MOUSE_PARAM_MAX as i64),
        (
            "MOUSE_PARAM_UTF8_MAX",
            rmux_util::key::MOUSE_PARAM_UTF8_MAX as i64,
        ),
        (
            "MOUSE_PARAM_BTN_OFF",
            rmux_util::key::MOUSE_PARAM_BTN_OFF as i64,
        ),
        (
            "MOUSE_PARAM_POS_OFF",
            rmux_util::key::MOUSE_PARAM_POS_OFF as i64,
        ),
        ("TTYC_ACSC", rmux_tty::term::TtyCodeCode::Acsc as i64),
        ("TTYC_AM", rmux_tty::term::TtyCodeCode::Am as i64),
        ("TTYC_AX", rmux_tty::term::TtyCodeCode::Ax as i64),
        ("TTYC_BCE", rmux_tty::term::TtyCodeCode::Bce as i64),
        ("TTYC_BEL", rmux_tty::term::TtyCodeCode::Bel as i64),
        ("TTYC_BIDI", rmux_tty::term::TtyCodeCode::Bidi as i64),
        ("TTYC_BLINK", rmux_tty::term::TtyCodeCode::Blink as i64),
        ("TTYC_BOLD", rmux_tty::term::TtyCodeCode::Bold as i64),
        ("TTYC_CIVIS", rmux_tty::term::TtyCodeCode::Civis as i64),
        ("TTYC_CLEAR", rmux_tty::term::TtyCodeCode::Clear as i64),
        ("TTYC_CLMG", rmux_tty::term::TtyCodeCode::Clmg as i64),
        ("TTYC_CMG", rmux_tty::term::TtyCodeCode::Cmg as i64),
        ("TTYC_CNORM", rmux_tty::term::TtyCodeCode::Cnorm as i64),
        ("TTYC_COLORS", rmux_tty::term::TtyCodeCode::Colors as i64),
        ("TTYC_CR", rmux_tty::term::TtyCodeCode::Cr as i64),
        ("TTYC_CS", rmux_tty::term::TtyCodeCode::Cs as i64),
        ("TTYC_CSR", rmux_tty::term::TtyCodeCode::Csr as i64),
        ("TTYC_CUB", rmux_tty::term::TtyCodeCode::Cub as i64),
        ("TTYC_CUB1", rmux_tty::term::TtyCodeCode::Cub1 as i64),
        ("TTYC_CUD", rmux_tty::term::TtyCodeCode::Cud as i64),
        ("TTYC_CUD1", rmux_tty::term::TtyCodeCode::Cud1 as i64),
        ("TTYC_CUF", rmux_tty::term::TtyCodeCode::Cuf as i64),
        ("TTYC_CUF1", rmux_tty::term::TtyCodeCode::Cuf1 as i64),
        ("TTYC_CUP", rmux_tty::term::TtyCodeCode::Cup as i64),
        ("TTYC_CUU", rmux_tty::term::TtyCodeCode::Cuu as i64),
        ("TTYC_CUU1", rmux_tty::term::TtyCodeCode::Cuu1 as i64),
        ("TTYC_CVVIS", rmux_tty::term::TtyCodeCode::Cvvis as i64),
        ("TTYC_DCH", rmux_tty::term::TtyCodeCode::Dch as i64),
        ("TTYC_DCH1", rmux_tty::term::TtyCodeCode::Dch1 as i64),
        ("TTYC_DIM", rmux_tty::term::TtyCodeCode::Dim as i64),
        ("TTYC_DL", rmux_tty::term::TtyCodeCode::Dl as i64),
        ("TTYC_DL1", rmux_tty::term::TtyCodeCode::Dl1 as i64),
        ("TTYC_DSBP", rmux_tty::term::TtyCodeCode::Dsbp as i64),
        ("TTYC_DSEKS", rmux_tty::term::TtyCodeCode::Dseks as i64),
        ("TTYC_DSESC", rmux_tty::term::TtyCodeCode::Dsesc as i64),
        ("TTYC_DSFCS", rmux_tty::term::TtyCodeCode::Dsfcs as i64),
        ("TTYC_DSMG", rmux_tty::term::TtyCodeCode::Dsmg as i64),
        ("TTYC_E3", rmux_tty::term::TtyCodeCode::E3 as i64),
        ("TTYC_ECH", rmux_tty::term::TtyCodeCode::Ech as i64),
        ("TTYC_ED", rmux_tty::term::TtyCodeCode::Ed as i64),
        ("TTYC_EL", rmux_tty::term::TtyCodeCode::El as i64),
        ("TTYC_EL1", rmux_tty::term::TtyCodeCode::El1 as i64),
        ("TTYC_ENACS", rmux_tty::term::TtyCodeCode::Enacs as i64),
        ("TTYC_ENBP", rmux_tty::term::TtyCodeCode::Enbp as i64),
        ("TTYC_ENEKS", rmux_tty::term::TtyCodeCode::Eneks as i64),
        ("TTYC_ENESC", rmux_tty::term::TtyCodeCode::Enesc as i64),
        ("TTYC_ENFCS", rmux_tty::term::TtyCodeCode::Enfcs as i64),
        ("TTYC_ENMG", rmux_tty::term::TtyCodeCode::Enmg as i64),
        ("TTYC_FSL", rmux_tty::term::TtyCodeCode::Fsl as i64),
        ("TTYC_HLS", rmux_tty::term::TtyCodeCode::Hls as i64),
        ("TTYC_HOME", rmux_tty::term::TtyCodeCode::Home as i64),
        ("TTYC_HPA", rmux_tty::term::TtyCodeCode::Hpa as i64),
        ("TTYC_ICH", rmux_tty::term::TtyCodeCode::Ich as i64),
        ("TTYC_ICH1", rmux_tty::term::TtyCodeCode::Ich1 as i64),
        ("TTYC_IL", rmux_tty::term::TtyCodeCode::Il as i64),
        ("TTYC_IL1", rmux_tty::term::TtyCodeCode::Il1 as i64),
        ("TTYC_IND", rmux_tty::term::TtyCodeCode::Ind as i64),
        ("TTYC_INDN", rmux_tty::term::TtyCodeCode::Indn as i64),
        ("TTYC_INVIS", rmux_tty::term::TtyCodeCode::Invis as i64),
        ("TTYC_KCBT", rmux_tty::term::TtyCodeCode::Kcbt as i64),
        ("TTYC_KCUB1", rmux_tty::term::TtyCodeCode::Kcub1 as i64),
        ("TTYC_KCUD1", rmux_tty::term::TtyCodeCode::Kcud1 as i64),
        ("TTYC_KCUF1", rmux_tty::term::TtyCodeCode::Kcuf1 as i64),
        ("TTYC_KCUU1", rmux_tty::term::TtyCodeCode::Kcuu1 as i64),
        ("TTYC_KDC2", rmux_tty::term::TtyCodeCode::Kdc2 as i64),
        ("TTYC_KDC3", rmux_tty::term::TtyCodeCode::Kdc3 as i64),
        ("TTYC_KDC4", rmux_tty::term::TtyCodeCode::Kdc4 as i64),
        ("TTYC_KDC5", rmux_tty::term::TtyCodeCode::Kdc5 as i64),
        ("TTYC_KDC6", rmux_tty::term::TtyCodeCode::Kdc6 as i64),
        ("TTYC_KDC7", rmux_tty::term::TtyCodeCode::Kdc7 as i64),
        ("TTYC_KDCH1", rmux_tty::term::TtyCodeCode::Kdch1 as i64),
        ("TTYC_KDN2", rmux_tty::term::TtyCodeCode::Kdn2 as i64),
        ("TTYC_KDN3", rmux_tty::term::TtyCodeCode::Kdn3 as i64),
        ("TTYC_KDN4", rmux_tty::term::TtyCodeCode::Kdn4 as i64),
        ("TTYC_KDN5", rmux_tty::term::TtyCodeCode::Kdn5 as i64),
        ("TTYC_KDN6", rmux_tty::term::TtyCodeCode::Kdn6 as i64),
        ("TTYC_KDN7", rmux_tty::term::TtyCodeCode::Kdn7 as i64),
        ("TTYC_KEND", rmux_tty::term::TtyCodeCode::Kend as i64),
        ("TTYC_KEND2", rmux_tty::term::TtyCodeCode::Kend2 as i64),
        ("TTYC_KEND3", rmux_tty::term::TtyCodeCode::Kend3 as i64),
        ("TTYC_KEND4", rmux_tty::term::TtyCodeCode::Kend4 as i64),
        ("TTYC_KEND5", rmux_tty::term::TtyCodeCode::Kend5 as i64),
        ("TTYC_KEND6", rmux_tty::term::TtyCodeCode::Kend6 as i64),
        ("TTYC_KEND7", rmux_tty::term::TtyCodeCode::Kend7 as i64),
        ("TTYC_KF1", rmux_tty::term::TtyCodeCode::Kf1 as i64),
        ("TTYC_KF10", rmux_tty::term::TtyCodeCode::Kf10 as i64),
        ("TTYC_KF11", rmux_tty::term::TtyCodeCode::Kf11 as i64),
        ("TTYC_KF12", rmux_tty::term::TtyCodeCode::Kf12 as i64),
        ("TTYC_KF13", rmux_tty::term::TtyCodeCode::Kf13 as i64),
        ("TTYC_KF14", rmux_tty::term::TtyCodeCode::Kf14 as i64),
        ("TTYC_KF15", rmux_tty::term::TtyCodeCode::Kf15 as i64),
        ("TTYC_KF16", rmux_tty::term::TtyCodeCode::Kf16 as i64),
        ("TTYC_KF17", rmux_tty::term::TtyCodeCode::Kf17 as i64),
        ("TTYC_KF18", rmux_tty::term::TtyCodeCode::Kf18 as i64),
        ("TTYC_KF19", rmux_tty::term::TtyCodeCode::Kf19 as i64),
        ("TTYC_KF2", rmux_tty::term::TtyCodeCode::Kf2 as i64),
        ("TTYC_KF20", rmux_tty::term::TtyCodeCode::Kf20 as i64),
        ("TTYC_KF21", rmux_tty::term::TtyCodeCode::Kf21 as i64),
        ("TTYC_KF22", rmux_tty::term::TtyCodeCode::Kf22 as i64),
        ("TTYC_KF23", rmux_tty::term::TtyCodeCode::Kf23 as i64),
        ("TTYC_KF24", rmux_tty::term::TtyCodeCode::Kf24 as i64),
        ("TTYC_KF25", rmux_tty::term::TtyCodeCode::Kf25 as i64),
        ("TTYC_KF26", rmux_tty::term::TtyCodeCode::Kf26 as i64),
        ("TTYC_KF27", rmux_tty::term::TtyCodeCode::Kf27 as i64),
        ("TTYC_KF28", rmux_tty::term::TtyCodeCode::Kf28 as i64),
        ("TTYC_KF29", rmux_tty::term::TtyCodeCode::Kf29 as i64),
        ("TTYC_KF3", rmux_tty::term::TtyCodeCode::Kf3 as i64),
        ("TTYC_KF30", rmux_tty::term::TtyCodeCode::Kf30 as i64),
        ("TTYC_KF31", rmux_tty::term::TtyCodeCode::Kf31 as i64),
        ("TTYC_KF32", rmux_tty::term::TtyCodeCode::Kf32 as i64),
        ("TTYC_KF33", rmux_tty::term::TtyCodeCode::Kf33 as i64),
        ("TTYC_KF34", rmux_tty::term::TtyCodeCode::Kf34 as i64),
        ("TTYC_KF35", rmux_tty::term::TtyCodeCode::Kf35 as i64),
        ("TTYC_KF36", rmux_tty::term::TtyCodeCode::Kf36 as i64),
        ("TTYC_KF37", rmux_tty::term::TtyCodeCode::Kf37 as i64),
        ("TTYC_KF38", rmux_tty::term::TtyCodeCode::Kf38 as i64),
        ("TTYC_KF39", rmux_tty::term::TtyCodeCode::Kf39 as i64),
        ("TTYC_KF4", rmux_tty::term::TtyCodeCode::Kf4 as i64),
        ("TTYC_KF40", rmux_tty::term::TtyCodeCode::Kf40 as i64),
        ("TTYC_KF41", rmux_tty::term::TtyCodeCode::Kf41 as i64),
        ("TTYC_KF42", rmux_tty::term::TtyCodeCode::Kf42 as i64),
        ("TTYC_KF43", rmux_tty::term::TtyCodeCode::Kf43 as i64),
        ("TTYC_KF44", rmux_tty::term::TtyCodeCode::Kf44 as i64),
        ("TTYC_KF45", rmux_tty::term::TtyCodeCode::Kf45 as i64),
        ("TTYC_KF46", rmux_tty::term::TtyCodeCode::Kf46 as i64),
        ("TTYC_KF47", rmux_tty::term::TtyCodeCode::Kf47 as i64),
        ("TTYC_KF48", rmux_tty::term::TtyCodeCode::Kf48 as i64),
        ("TTYC_KF49", rmux_tty::term::TtyCodeCode::Kf49 as i64),
        ("TTYC_KF5", rmux_tty::term::TtyCodeCode::Kf5 as i64),
        ("TTYC_KF50", rmux_tty::term::TtyCodeCode::Kf50 as i64),
        ("TTYC_KF51", rmux_tty::term::TtyCodeCode::Kf51 as i64),
        ("TTYC_KF52", rmux_tty::term::TtyCodeCode::Kf52 as i64),
        ("TTYC_KF53", rmux_tty::term::TtyCodeCode::Kf53 as i64),
        ("TTYC_KF54", rmux_tty::term::TtyCodeCode::Kf54 as i64),
        ("TTYC_KF55", rmux_tty::term::TtyCodeCode::Kf55 as i64),
        ("TTYC_KF56", rmux_tty::term::TtyCodeCode::Kf56 as i64),
        ("TTYC_KF57", rmux_tty::term::TtyCodeCode::Kf57 as i64),
        ("TTYC_KF58", rmux_tty::term::TtyCodeCode::Kf58 as i64),
        ("TTYC_KF59", rmux_tty::term::TtyCodeCode::Kf59 as i64),
        ("TTYC_KF6", rmux_tty::term::TtyCodeCode::Kf6 as i64),
        ("TTYC_KF60", rmux_tty::term::TtyCodeCode::Kf60 as i64),
        ("TTYC_KF61", rmux_tty::term::TtyCodeCode::Kf61 as i64),
        ("TTYC_KF62", rmux_tty::term::TtyCodeCode::Kf62 as i64),
        ("TTYC_KF63", rmux_tty::term::TtyCodeCode::Kf63 as i64),
        ("TTYC_KF7", rmux_tty::term::TtyCodeCode::Kf7 as i64),
        ("TTYC_KF8", rmux_tty::term::TtyCodeCode::Kf8 as i64),
        ("TTYC_KF9", rmux_tty::term::TtyCodeCode::Kf9 as i64),
        ("TTYC_KHOM2", rmux_tty::term::TtyCodeCode::Khom2 as i64),
        ("TTYC_KHOM3", rmux_tty::term::TtyCodeCode::Khom3 as i64),
        ("TTYC_KHOM4", rmux_tty::term::TtyCodeCode::Khom4 as i64),
        ("TTYC_KHOM5", rmux_tty::term::TtyCodeCode::Khom5 as i64),
        ("TTYC_KHOM6", rmux_tty::term::TtyCodeCode::Khom6 as i64),
        ("TTYC_KHOM7", rmux_tty::term::TtyCodeCode::Khom7 as i64),
        ("TTYC_KHOME", rmux_tty::term::TtyCodeCode::Khome as i64),
        ("TTYC_KIC2", rmux_tty::term::TtyCodeCode::Kic2 as i64),
        ("TTYC_KIC3", rmux_tty::term::TtyCodeCode::Kic3 as i64),
        ("TTYC_KIC4", rmux_tty::term::TtyCodeCode::Kic4 as i64),
        ("TTYC_KIC5", rmux_tty::term::TtyCodeCode::Kic5 as i64),
        ("TTYC_KIC6", rmux_tty::term::TtyCodeCode::Kic6 as i64),
        ("TTYC_KIC7", rmux_tty::term::TtyCodeCode::Kic7 as i64),
        ("TTYC_KICH1", rmux_tty::term::TtyCodeCode::Kich1 as i64),
        ("TTYC_KIND", rmux_tty::term::TtyCodeCode::Kind as i64),
        ("TTYC_KLFT2", rmux_tty::term::TtyCodeCode::Klft2 as i64),
        ("TTYC_KLFT3", rmux_tty::term::TtyCodeCode::Klft3 as i64),
        ("TTYC_KLFT4", rmux_tty::term::TtyCodeCode::Klft4 as i64),
        ("TTYC_KLFT5", rmux_tty::term::TtyCodeCode::Klft5 as i64),
        ("TTYC_KLFT6", rmux_tty::term::TtyCodeCode::Klft6 as i64),
        ("TTYC_KLFT7", rmux_tty::term::TtyCodeCode::Klft7 as i64),
        ("TTYC_KMOUS", rmux_tty::term::TtyCodeCode::Kmous as i64),
        ("TTYC_KNP", rmux_tty::term::TtyCodeCode::Knp as i64),
        ("TTYC_KNXT2", rmux_tty::term::TtyCodeCode::Knxt2 as i64),
        ("TTYC_KNXT3", rmux_tty::term::TtyCodeCode::Knxt3 as i64),
        ("TTYC_KNXT4", rmux_tty::term::TtyCodeCode::Knxt4 as i64),
        ("TTYC_KNXT5", rmux_tty::term::TtyCodeCode::Knxt5 as i64),
        ("TTYC_KNXT6", rmux_tty::term::TtyCodeCode::Knxt6 as i64),
        ("TTYC_KNXT7", rmux_tty::term::TtyCodeCode::Knxt7 as i64),
        ("TTYC_KPP", rmux_tty::term::TtyCodeCode::Kpp as i64),
        ("TTYC_KPRV2", rmux_tty::term::TtyCodeCode::Kprv2 as i64),
        ("TTYC_KPRV3", rmux_tty::term::TtyCodeCode::Kprv3 as i64),
        ("TTYC_KPRV4", rmux_tty::term::TtyCodeCode::Kprv4 as i64),
        ("TTYC_KPRV5", rmux_tty::term::TtyCodeCode::Kprv5 as i64),
        ("TTYC_KPRV6", rmux_tty::term::TtyCodeCode::Kprv6 as i64),
        ("TTYC_KPRV7", rmux_tty::term::TtyCodeCode::Kprv7 as i64),
        ("TTYC_KRI", rmux_tty::term::TtyCodeCode::Kri as i64),
        ("TTYC_KRIT2", rmux_tty::term::TtyCodeCode::Krit2 as i64),
        ("TTYC_KRIT3", rmux_tty::term::TtyCodeCode::Krit3 as i64),
        ("TTYC_KRIT4", rmux_tty::term::TtyCodeCode::Krit4 as i64),
        ("TTYC_KRIT5", rmux_tty::term::TtyCodeCode::Krit5 as i64),
        ("TTYC_KRIT6", rmux_tty::term::TtyCodeCode::Krit6 as i64),
        ("TTYC_KRIT7", rmux_tty::term::TtyCodeCode::Krit7 as i64),
        ("TTYC_KUP2", rmux_tty::term::TtyCodeCode::Kup2 as i64),
        ("TTYC_KUP3", rmux_tty::term::TtyCodeCode::Kup3 as i64),
        ("TTYC_KUP4", rmux_tty::term::TtyCodeCode::Kup4 as i64),
        ("TTYC_KUP5", rmux_tty::term::TtyCodeCode::Kup5 as i64),
        ("TTYC_KUP6", rmux_tty::term::TtyCodeCode::Kup6 as i64),
        ("TTYC_KUP7", rmux_tty::term::TtyCodeCode::Kup7 as i64),
        ("TTYC_MS", rmux_tty::term::TtyCodeCode::Ms as i64),
        ("TTYC_NOBR", rmux_tty::term::TtyCodeCode::Nobr as i64),
        ("TTYC_OL", rmux_tty::term::TtyCodeCode::Ol as i64),
        ("TTYC_OP", rmux_tty::term::TtyCodeCode::Op as i64),
        ("TTYC_RECT", rmux_tty::term::TtyCodeCode::Rect as i64),
        ("TTYC_REV", rmux_tty::term::TtyCodeCode::Rev as i64),
        ("TTYC_RGB", rmux_tty::term::TtyCodeCode::Rgb as i64),
        ("TTYC_RI", rmux_tty::term::TtyCodeCode::Ri as i64),
        ("TTYC_RIN", rmux_tty::term::TtyCodeCode::Rin as i64),
        ("TTYC_RMACS", rmux_tty::term::TtyCodeCode::Rmacs as i64),
        ("TTYC_RMCUP", rmux_tty::term::TtyCodeCode::Rmcup as i64),
        ("TTYC_RMKX", rmux_tty::term::TtyCodeCode::Rmkx as i64),
        ("TTYC_SE", rmux_tty::term::TtyCodeCode::Se as i64),
        ("TTYC_SETAB", rmux_tty::term::TtyCodeCode::Setab as i64),
        ("TTYC_SETAF", rmux_tty::term::TtyCodeCode::Setaf as i64),
        ("TTYC_SETAL", rmux_tty::term::TtyCodeCode::Setal as i64),
        ("TTYC_SETRGBB", rmux_tty::term::TtyCodeCode::Setrgbb as i64),
        ("TTYC_SETRGBF", rmux_tty::term::TtyCodeCode::Setrgbf as i64),
        ("TTYC_SETULC", rmux_tty::term::TtyCodeCode::Setulc as i64),
        ("TTYC_SETULC1", rmux_tty::term::TtyCodeCode::Setulc1 as i64),
        ("TTYC_SGR0", rmux_tty::term::TtyCodeCode::Sgr0 as i64),
        ("TTYC_SITM", rmux_tty::term::TtyCodeCode::Sitm as i64),
        ("TTYC_SMACS", rmux_tty::term::TtyCodeCode::Smacs as i64),
        ("TTYC_SMCUP", rmux_tty::term::TtyCodeCode::Smcup as i64),
        ("TTYC_SMKX", rmux_tty::term::TtyCodeCode::Smkx as i64),
        ("TTYC_SMOL", rmux_tty::term::TtyCodeCode::Smol as i64),
        ("TTYC_SMSO", rmux_tty::term::TtyCodeCode::Smso as i64),
        ("TTYC_SMUL", rmux_tty::term::TtyCodeCode::Smul as i64),
        ("TTYC_SMULX", rmux_tty::term::TtyCodeCode::Smulx as i64),
        ("TTYC_SMXX", rmux_tty::term::TtyCodeCode::Smxx as i64),
        ("TTYC_SPB", rmux_tty::term::TtyCodeCode::Spb as i64),
        ("TTYC_SXL", rmux_tty::term::TtyCodeCode::Sxl as i64),
        ("TTYC_SS", rmux_tty::term::TtyCodeCode::Ss as i64),
        ("TTYC_SWD", rmux_tty::term::TtyCodeCode::Swd as i64),
        ("TTYC_SYNC", rmux_tty::term::TtyCodeCode::Sync as i64),
        ("TTYC_TC", rmux_tty::term::TtyCodeCode::Tc as i64),
        ("TTYC_TSL", rmux_tty::term::TtyCodeCode::Tsl as i64),
        ("TTYC_U8", rmux_tty::term::TtyCodeCode::U8 as i64),
        ("TTYC_VPA", rmux_tty::term::TtyCodeCode::Vpa as i64),
        ("TTYC_XT", rmux_tty::term::TtyCodeCode::Xt as i64),
    ];
    let mut c = String::from("#include \"tmux.h\"\nint main(void) {\n");
    for (name, _) in values {
        c.push_str(&format!("printf(\"%lld\\n\", (long long)({name}));\n"));
    }
    c.push_str("}\n");
    fs::write(dir.join("values.c"), c).unwrap();
    let mut cc = Command::new("cc");
    cc.args([
        "-DHAVE_CLOCK_GETTIME",
        "-DHAVE_EVENT2_EVENT_H",
        "-DHAVE_SYS_QUEUE_H",
        "-DHAVE_U_INT",
        "-DHAVE_U_CHAR",
    ]);
    if cfg!(target_os = "macos") {
        cc.args([
            "-DHAVE_SYS_TREE_H",
            "-DHAVE_BITSTRING_H",
            "-I/opt/homebrew/opt/libevent/include",
        ]);
    }
    let result = match cc
        .arg("-I")
        .arg(&dir)
        .arg(dir.join("values.c"))
        .arg("-o")
        .arg(dir.join("values"))
        .output()
    {
        Ok(result) => result,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIP pinned header comparison: cc unavailable");
            fs::remove_dir_all(dir).unwrap();
            return;
        }
        Err(error) => panic!("C compiler: {error}"),
    };
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(dir.join("values")).output().unwrap();
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    let actual: Vec<i64> = output.lines().map(|line| line.parse().unwrap()).collect();
    assert_eq!(actual.len(), values.len());
    for ((name, expected), actual) in values.iter().zip(actual) {
        assert_eq!(*expected, actual, "{name}");
    }
    fs::remove_dir_all(dir).unwrap();
}
