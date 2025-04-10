// This is a copypaste of
// https://gitlab.com/davidbittner/ansi-parser/-/blob/master/src/parsers.rs?ref_type=heads slightly
// modified to work directly in byte buffers

use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::{digit0, digit1};
use nom::combinator::opt;
use nom::sequence::{delimited, preceded};
use nom::{IResult, Parser};

macro_rules! tag_parser {
    ($sig:ident, $tag:expr, $ret:expr) => {
        fn $sig(input: &[u8]) -> IResult<&[u8], AnsiCode> {
            tag($tag)(input).map(|(s, _)| (s, $ret))
        }
    };
}

#[derive(Debug)]
pub enum AnsiCode {
    Escape,
    CursorPos(u32, u32),
    CursorUp(u32),
    CursorDown(u32),
    CursorForward(u32),
    CursorBackward(u32),
    CursorResetStyle,
    CursorSave,
    CursorRestore,
    EnableCursorBlink,
    DisableCursorBlink,
    EraseDisplay,
    EraseAllDisplay,
    EraseLine,
    SetGraphicsMode(u8, [u8; 5]),
    SetMode(u8),
    ResetMode(u8),
    HideCursor,
    ShowCursor,
    CursorToApp,
    SetNewLineMode,
    SetCol132,
    SetSmoothScroll,
    SetReverseVideo,
    SetOriginRelative,
    SetAutoWrap,
    SetAutoRepeat,
    SetInterlacing,
    SetLineFeedMode,
    SetCursorKeyToCursor,
    SetVT52,
    SetCol80,
    SetJumpScrolling,
    SetNormalVideo,
    SetOriginAbsolute,
    ResetAutoWrap,
    ResetAutoRepeat,
    ResetInterlacing,
    SetAlternateKeypad,
    SetNumericKeypad,
    SetUKG0,
    SetUKG1,
    SetUSG0,
    SetUSG1,
    SetG0SpecialChars,
    SetG1SpecialChars,
    SetG0AlternateChar,
    SetG1AlternateChar,
    SetG0AltAndSpecialGraph,
    SetG1AltAndSpecialGraph,
    SetSingleShift2,
    SetSingleShift3,
    SetTopAndBottom(u32, u32),
    EnableBracketedPaste,
    DisableBracketedPaste,
}

#[derive(Debug)]
pub struct AnsiParser<'a> {
    slice: &'a [u8],
}

impl<'a> AnsiParser<'a> {
    pub fn new(slice: &'a [u8]) -> Self {
        Self { slice }
    }
}

#[derive(Debug)]
pub enum Output<'a> {
    Bytes(&'a [u8]),
    Escape(AnsiCode),
}

impl<'a> Iterator for AnsiParser<'a> {
    type Item = Output<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        fn find_in_slice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        }

        if self.slice.is_empty() {
            return None;
        }

        match find_in_slice(self.slice, b"\x1b") {
            Some(0) => {
                let res = ansi_parse(self.slice);

                if let Ok((rest, ac)) = res {
                    self.slice = rest;
                    Some(Output::Escape(ac))
                } else {
                    let pos = find_in_slice(&self.slice[1..], b"\x1b");
                    match pos {
                        Some(i) => {
                            let i = i + 1;
                            let bytes = &self.slice[..i];
                            self.slice = &self.slice[i..];
                            Some(Output::Bytes(bytes))
                        }

                        None => {
                            let bytes = self.slice;
                            self.slice = &[];

                            Some(Output::Bytes(bytes))
                        }
                    }
                }
            }
            Some(n) => {
                let bytes = &self.slice[..n];
                self.slice = &self.slice[n..];
                Some(Output::Bytes(bytes))
            }
            None => {
                let bytes = self.slice;
                self.slice = &[];

                Some(Output::Bytes(bytes))
            }
        }
    }
}

fn parse_def_cursor_int(input: &[u8]) -> IResult<&[u8], u32> {
    digit0(input).map(|(s, d)| {
        (
            s,
            std::str::from_utf8(d)
                .unwrap_or("1")
                .parse::<u32>()
                .unwrap_or(1),
        )
    })
}

fn parse_u8(input: &[u8]) -> IResult<&[u8], u8> {
    digit1(input).map(|(s, d)| {
        (
            s,
            std::str::from_utf8(d)
                .unwrap_or("1")
                .parse::<u8>()
                .unwrap_or(1),
        )
    })
}

fn parse_u32(input: &[u8]) -> IResult<&[u8], u32> {
    digit1(input).map(|(s, d)| {
        (
            s,
            std::str::from_utf8(d)
                .unwrap_or("1")
                .parse::<u32>()
                .unwrap_or(1),
        )
    })
}

fn escape(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    tag("\u{1b}")(input).map(|(s, _)| (s, AnsiCode::Escape))
}

fn cursor_up(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    delimited(tag("["), parse_def_cursor_int, tag("A"))
        .parse(input)
        .map(|(s, amount)| (s, AnsiCode::CursorUp(amount)))
}

fn cursor_down(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    delimited(tag("["), parse_def_cursor_int, tag("B"))
        .parse(input)
        .map(|(s, amount)| (s, AnsiCode::CursorUp(amount)))
}
fn cursor_forward(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    delimited(tag("["), parse_def_cursor_int, tag("C"))
        .parse(input)
        .map(|(s, amount)| (s, AnsiCode::CursorUp(amount)))
}
fn cursor_backward(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    delimited(tag("["), parse_def_cursor_int, tag("D"))
        .parse(input)
        .map(|(s, amount)| (s, AnsiCode::CursorUp(amount)))
}

fn set_mode(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (tag("[="), parse_u8, tag("h"))
        .parse(input)
        .map(|(s, (_, m, _))| (s, AnsiCode::SetMode(m)))
}

fn reset_mode(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (tag("[="), parse_u8, tag("l"))
        .parse(input)
        .map(|(s, (_, m, _))| (s, AnsiCode::ResetMode(m)))
}

fn graphics_mode1(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (tag("["), parse_u8, tag("m"))
        .parse(input)
        .map(|(s, (_, a, _))| (s, AnsiCode::SetGraphicsMode(1, [a, 0, 0, 0, 0])))
}

fn graphics_mode2(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (tag("["), parse_u8, tag(";"), parse_u8, tag("m"))
        .parse(input)
        .map(|(s, (_, a, _, b, _))| (s, AnsiCode::SetGraphicsMode(2, [a, b, 0, 0, 0])))
}
fn graphics_mode3(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (
        tag("["),
        parse_u8,
        tag(";"),
        parse_u8,
        tag(";"),
        parse_u8,
        tag("m"),
    )
        .parse(input)
        .map(|(s, (_, a, _, b, _, c, _))| (s, AnsiCode::SetGraphicsMode(3, [a, b, c, 0, 0])))
}

fn graphics_mode4(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (
        tag("["),
        parse_u8,
        tag(";"),
        parse_u8,
        tag(";"),
        parse_u8,
        tag(";"),
        parse_u8,
        tag("m"),
    )
        .parse(input)
        .map(|(s, (_, a, _, b, _, c, _, d, _))| (s, AnsiCode::SetGraphicsMode(4, [a, b, c, d, 0])))
}

fn graphics_mode5(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (
        tag("["),
        parse_u8,
        tag(";"),
        parse_u8,
        tag(";"),
        parse_u8,
        tag(";"),
        parse_u8,
        tag(";"),
        parse_u8,
        tag("m"),
    )
        .parse(input)
        .map(|(s, (_, a, _, b, _, c, _, d, _, e, _))| {
            (s, AnsiCode::SetGraphicsMode(5, [a, b, c, d, e]))
        })
}

fn set_top_and_bottom(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (tag("["), parse_u32, tag(";"), parse_u32, tag("r"))
        .parse(input)
        .map(|(s, (_, x, _, y, _))| (s, AnsiCode::SetTopAndBottom(x, y)))
}

fn cursor_pos(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    (
        tag("["),
        parse_def_cursor_int,
        opt(tag(";")),
        parse_def_cursor_int,
        alt((tag("H"), tag("f"))),
    )
        .parse(input)
        .map(|(s, (_, x, _, y, _))| (s, AnsiCode::CursorPos(x, y)))
}

fn graphics_mode(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    alt((
        graphics_mode1,
        graphics_mode2,
        graphics_mode3,
        graphics_mode4,
        graphics_mode5,
    ))
    .parse(input)
}

tag_parser!(cursor_reset_style, "[m", AnsiCode::CursorResetStyle);
tag_parser!(cursor_save, "[s", AnsiCode::CursorSave);
tag_parser!(cursor_restore, "[u", AnsiCode::CursorRestore);
tag_parser!(erase_in_display, "[J", AnsiCode::EraseDisplay);
tag_parser!(erase_full_display, "[2J", AnsiCode::EraseAllDisplay);
tag_parser!(erase_line, "[K", AnsiCode::EraseLine);
tag_parser!(enable_bracketed_paste, "[?2004h", AnsiCode::EnableBracketedPaste);
tag_parser!(disable_bracketed_paste, "[?2004l", AnsiCode::DisableBracketedPaste);
tag_parser!(enable_cursor_blink, "[?12h", AnsiCode::EnableCursorBlink);
tag_parser!(disable_cursor_blink, "[?12l", AnsiCode::DisableCursorBlink);
tag_parser!(hide_cursor, "[?25l", AnsiCode::HideCursor);
tag_parser!(show_cursor, "[?25h", AnsiCode::ShowCursor);
tag_parser!(cursor_to_app, "[?1h", AnsiCode::CursorToApp);
tag_parser!(set_new_line_mode, "[20h", AnsiCode::SetNewLineMode);
tag_parser!(set_col_132, "[?3h", AnsiCode::SetCol132);
tag_parser!(set_smooth_scroll, "[?4h", AnsiCode::SetSmoothScroll);
tag_parser!(set_reverse_video, "[?5h", AnsiCode::SetReverseVideo);
tag_parser!(set_origin_rel, "[?6h", AnsiCode::SetOriginRelative);
tag_parser!(set_auto_wrap, "[?7h", AnsiCode::SetAutoWrap);
tag_parser!(set_auto_repeat, "[?8h", AnsiCode::SetAutoRepeat);
tag_parser!(set_interlacing, "[?9h", AnsiCode::SetInterlacing);
tag_parser!(set_linefeed, "[20l", AnsiCode::SetLineFeedMode);
tag_parser!(set_cursorkey, "[?1l", AnsiCode::SetCursorKeyToCursor);
tag_parser!(set_vt52, "[?2l", AnsiCode::SetVT52);
tag_parser!(set_col80, "[?3l", AnsiCode::SetCol80);
tag_parser!(set_jump_scroll, "[?4l", AnsiCode::SetJumpScrolling);
tag_parser!(set_normal_video, "[?5l", AnsiCode::SetNormalVideo);
tag_parser!(set_origin_abs, "[?6l", AnsiCode::SetOriginAbsolute);
tag_parser!(reset_auto_wrap, "[?7l", AnsiCode::ResetAutoWrap);
tag_parser!(reset_auto_repeat, "[?8l", AnsiCode::ResetAutoRepeat);
tag_parser!(reset_interlacing, "[?9l", AnsiCode::ResetInterlacing);

tag_parser!(set_alternate_keypad, "=", AnsiCode::SetAlternateKeypad);
tag_parser!(set_numeric_keypad, ">", AnsiCode::SetNumericKeypad);
tag_parser!(set_uk_g0, "(A", AnsiCode::SetUKG0);
tag_parser!(set_uk_g1, ")A", AnsiCode::SetUKG1);
tag_parser!(set_us_g0, "(B", AnsiCode::SetUSG0);
tag_parser!(set_us_g1, ")B", AnsiCode::SetUSG1);
tag_parser!(set_g0_special, "(0", AnsiCode::SetG0SpecialChars);
tag_parser!(set_g1_special, ")0", AnsiCode::SetG1SpecialChars);
tag_parser!(set_g0_alternate, "(1", AnsiCode::SetG0AlternateChar);
tag_parser!(set_g1_alternate, ")1", AnsiCode::SetG1AlternateChar);
tag_parser!(set_g0_graph, "(2", AnsiCode::SetG0AltAndSpecialGraph);
tag_parser!(set_g1_graph, ")2", AnsiCode::SetG1AltAndSpecialGraph);
tag_parser!(set_single_shift2, "N", AnsiCode::SetSingleShift2);
tag_parser!(set_single_shift3, "O", AnsiCode::SetSingleShift3);

pub fn body(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    // `alt` only supports up to 21 parsers, and nom doesn't seem to
    // have an alternative with higher variability.
    // So we simply nest them.
    alt((
        alt((
            escape,
            cursor_pos,
            cursor_up,
            cursor_down,
            cursor_forward,
            cursor_backward,
            cursor_save,
            cursor_restore,
            erase_full_display,
            erase_in_display,
            erase_line,
            graphics_mode,
            set_mode,
            reset_mode,
            hide_cursor,
            show_cursor,
            cursor_to_app,
            set_new_line_mode,
            set_col_132,
            set_smooth_scroll,
            set_reverse_video,
        )),
        alt((
            set_auto_wrap,
            set_origin_rel,
            set_auto_repeat,
            set_interlacing,
            set_linefeed,
            set_cursorkey,
            set_vt52,
            set_col80,
            set_jump_scroll,
            set_normal_video,
            set_origin_abs,
            reset_auto_wrap,
            reset_auto_repeat,
            reset_interlacing,
            set_top_and_bottom,
            set_alternate_keypad,
            set_numeric_keypad,
            set_uk_g0,
            set_uk_g1,
            set_us_g0,
            set_us_g1,
        )),
        set_g0_special,
        set_g1_special,
        set_g0_alternate,
        set_g1_alternate,
        set_g0_graph,
        set_g1_graph,
        set_single_shift2,
        set_single_shift3,
        enable_bracketed_paste,
        disable_bracketed_paste,
        enable_cursor_blink,
        disable_cursor_blink,
        cursor_reset_style,
    ))
    .parse(input)
}

pub fn ansi_parse(input: &[u8]) -> IResult<&[u8], AnsiCode> {
    preceded(tag("\u{1b}"), body).parse(input)
}
