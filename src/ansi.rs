//! A command's output as a terminal shows it: the colours and weights its ANSI escape sequences
//! (ECMA-48 SGR, `ESC [ … m`) ask for, over the text without the escapes. Other escape sequences
//! (cursor moves, window titles, hyperlinks' targets) are dropped, a carriage return starts its
//! line again as a progress bar expects, and other control characters are left out.

use std::ops::Range;

/// A colour an escape names. The view picks the actual colour for the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// One of the 256 indexed colours: 0 to 7 normal, 8 to 15 bright, then the 6×6×6 cube and
    /// the grey ramp.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// How a run of text is drawn. The default is the terminal's own text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// Output without its escapes, and the byte ranges of it drawn in other than the default style,
/// in order and not overlapping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Styled {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
}

impl Styled {
    fn push(&mut self, c: char, style: Style) {
        let start = self.text.len();
        self.text.push(c);
        if style == Style::default() {
            return;
        }
        match self.runs.last_mut() {
            Some((range, last)) if range.end == start && *last == style => {
                range.end += c.len_utf8()
            }
            _ => self.runs.push((start..self.text.len(), style)),
        }
    }

    /// Drops the text after byte `at`, and the runs over it.
    fn truncate(&mut self, at: usize) {
        self.text.truncate(at);
        self.runs.retain_mut(|(range, _)| {
            range.end = range.end.min(at);
            range.start < range.end
        });
    }
}

/// `text` as a terminal shows it.
pub fn parse(text: &str) -> Styled {
    let mut out = Styled {
        text: String::with_capacity(text.len()),
        runs: Vec::new(),
    };
    let mut style = Style::default();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    let mut end = None;
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            end = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    if end == Some('m') {
                        sgr(&params, &mut style);
                    }
                }
                // An operating system command (a title, a hyperlink's target) runs to BEL or
                // ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                // Any other escape is two characters, or three for a character set choice.
                Some('(' | ')' | '*' | '+') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' if chars.peek() == Some(&'\n') => {}
            // A carriage return draws the line again from its start.
            '\r' => {
                let start = out.text.rfind('\n').map_or(0, |i| i + 1);
                out.truncate(start);
            }
            '\u{8}' => {
                let line_start = out.text.rfind('\n').map_or(0, |i| i + 1);
                if let Some((at, _)) = out.text[line_start..].char_indices().next_back() {
                    out.truncate(line_start + at);
                }
            }
            '\n' | '\t' => out.push(c, style),
            c if c.is_control() => {}
            c => out.push(c, style),
        }
    }
    out
}

/// Applies one SGR sequence's parameters, such as `1;31` or `38;5;208`, to `style`.
fn sgr(params: &str, style: &mut Style) {
    let mut codes = params.split([';', ':']).map(|p| {
        if p.is_empty() {
            0
        } else {
            p.parse::<u16>().unwrap_or(u16::MAX)
        }
    });
    // An empty sequence (`ESC [ m`) resets, as `0` does.
    if params.is_empty() {
        *style = Style::default();
        return;
    }
    while let Some(code) = codes.next() {
        match code {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.inverse = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.inverse = false,
            30..=37 => style.fg = Some(Color::Indexed((code - 30) as u8)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(Color::Indexed((code - 40) as u8)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(Color::Indexed((code - 90 + 8) as u8)),
            100..=107 => style.bg = Some(Color::Indexed((code - 100 + 8) as u8)),
            38 | 48 => {
                let color = match codes.next() {
                    Some(5) => codes
                        .next()
                        .and_then(|n| u8::try_from(n).ok())
                        .map(Color::Indexed),
                    Some(2) => {
                        let mut c = || codes.next().and_then(|n| u8::try_from(n).ok());
                        match (c(), c(), c()) {
                            (Some(r), Some(g), Some(b)) => Some(Color::Rgb(r, g, b)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if code == 38 {
                    style.fg = color;
                } else {
                    style.bg = color;
                }
            }
            _ => {}
        }
    }
}

/// The red, green and blue of an indexed colour past the first sixteen, which have no fixed
/// look: the 6×6×6 cube from 16, then 24 greys from 232, as xterm draws them.
pub fn cube(n: u8) -> Option<(u8, u8, u8)> {
    match n {
        0..=15 => None,
        16..=231 => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let n = n - 16;
            Some((level(n / 36), level(n / 6 % 6), level(n % 6)))
        }
        _ => {
            let grey = 8 + (n - 232) * 10;
            Some((grey, grey, grey))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> Style {
        Style {
            fg: Some(Color::Indexed(1)),
            ..Style::default()
        }
    }

    #[test]
    fn colours_become_runs_over_the_text_without_escapes() {
        let out = parse("\u{1b}[31mred\u{1b}[0m plain \u{1b}[1;38;5;208mbold\u{1b}[m");
        assert_eq!(out.text, "red plain bold");
        assert_eq!(out.runs[0], (0..3, red()));
        assert_eq!(
            out.runs[1],
            (
                10..14,
                Style {
                    fg: Some(Color::Indexed(208)),
                    bold: true,
                    ..Style::default()
                }
            )
        );
        assert_eq!(out.runs.len(), 2);
        let rgb = parse("\u{1b}[48;2;10;20;30mx");
        assert_eq!(rgb.runs[0].1.bg, Some(Color::Rgb(10, 20, 30)));
        let bright = parse("\u{1b}[91mx\u{1b}[39my");
        assert_eq!(
            bright.runs,
            vec![(
                0..1,
                Style {
                    fg: Some(Color::Indexed(9)),
                    ..Style::default()
                }
            )]
        );
    }

    #[test]
    fn other_escapes_and_controls_are_dropped() {
        let out =
            parse("\u{1b}]8;;https://x.test\u{7}link\u{1b}]8;;\u{1b}\\ \u{1b}[2Kdone\u{7}\r\n");
        assert_eq!(out.text, "link done\n");
        assert!(out.runs.is_empty());
    }

    #[test]
    fn a_carriage_return_redraws_its_line_and_a_backspace_erases() {
        let out = parse("first\n\u{1b}[31m 10%\r 50%\r100%\u{1b}[0m\nab\u{8}c");
        assert_eq!(out.text, "first\n100%\nac");
        assert_eq!(out.runs, vec![(6..10, red())]);
    }

    #[test]
    fn the_cube_and_greys_have_their_xterm_values() {
        assert_eq!(cube(1), None);
        assert_eq!(cube(16), Some((0, 0, 0)));
        assert_eq!(cube(208), Some((255, 135, 0)));
        assert_eq!(cube(232), Some((8, 8, 8)));
        assert_eq!(cube(255), Some((238, 238, 238)));
    }
}
