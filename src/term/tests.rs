//! VT-compatibility, Unicode, resize, selection, search and stress tests for the screen model.
//! Everything here runs without a window or a PTY.

use super::*;
use mitos_utils::ipc::RichWidget;

fn t(cols: usize, rows: usize) -> Term {
    Term::new(cols, rows, 1000)
}

fn feed(t: &mut Term, s: &str) {
    t.process(s.as_bytes());
}

fn row(t: &Term, y: usize) -> String {
    t.screen_row_text(y)
}

fn cell(t: &Term, x: usize, y: usize) -> Cell {
    t.view_row(y).cells[x]
}

fn replies(t: &mut Term) -> Vec<String> {
    t.take_events()
        .into_iter()
        .filter_map(|e| match e {
            TermEvent::PtyWrite(b) => Some(String::from_utf8_lossy(&b).into_owned()),
            _ => None,
        })
        .collect()
}

/// Structural invariants that must hold after *any* input.
fn check_invariants(t: &Term) {
    assert_eq!(t.grid.len(), t.rows, "grid rows");
    for (y, r) in t.grid.iter().enumerate() {
        assert_eq!(r.cells.len(), t.cols, "row {y} width");
        for (x, c) in r.cells.iter().enumerate() {
            if c.flags & attr::WIDE != 0 {
                assert!(x + 1 < t.cols, "wide head in last column");
                assert!(r.cells[x + 1].flags & attr::WIDE_TAIL != 0, "wide head without tail at {x},{y}");
            }
            if c.flags & attr::WIDE_TAIL != 0 {
                assert!(x > 0 && r.cells[x - 1].flags & attr::WIDE != 0, "orphan wide tail at {x},{y}");
            }
        }
    }
    assert!(t.cursor.x < t.cols, "cursor x {} >= {}", t.cursor.x, t.cols);
    assert!(t.cursor.y < t.rows, "cursor y {} >= {}", t.cursor.y, t.rows);
    assert!(t.scroll_top <= t.scroll_bottom && t.scroll_bottom < t.rows);
    assert_eq!(t.tabs.len(), t.cols);
    assert!(t.scrollback.len() <= t.scrollback_limit.max(0));
    assert!(t.display_offset <= t.scrollback.len());
}

// ---------------------------------------------------------------------------
// Basics
// ---------------------------------------------------------------------------

#[test]
fn prints_text_and_moves_cursor() {
    let mut t = t(20, 5);
    feed(&mut t, "hello");
    assert_eq!(row(&t, 0), "hello");
    assert_eq!(t.cursor_pos(), (5, 0));
}

#[test]
fn crlf_and_linefeed() {
    let mut t = t(20, 5);
    feed(&mut t, "ab\r\ncd");
    assert_eq!(row(&t, 0), "ab");
    assert_eq!(row(&t, 1), "cd");
    assert_eq!(t.cursor_pos(), (2, 1));
    // bare LF keeps the column (ONLCR is the tty's job, not ours)
    let mut t2 = t2();
    feed(&mut t2, "ab\ncd");
    assert_eq!(row(&t2, 0), "ab");
    assert_eq!(row(&t2, 1), "  cd");
}

fn t2() -> Term {
    t(20, 5)
}

#[test]
fn deferred_wrap_semantics() {
    let mut t = t(5, 3);
    feed(&mut t, "abcde");
    assert_eq!(t.cursor_pos(), (4, 0));
    assert!(t.cursor_wrap_pending());
    assert_eq!(row(&t, 1), "");
    feed(&mut t, "f");
    assert_eq!(row(&t, 0), "abcde");
    assert_eq!(row(&t, 1), "f");
    assert!(t.row_by_abs(t.screen_top_abs()).unwrap().wrapped);
    check_invariants(&t);
}

#[test]
fn autowrap_off_overwrites_last_column() {
    let mut t = t(5, 3);
    feed(&mut t, "\x1b[?7labcdefg");
    assert_eq!(row(&t, 0), "abcdg");
    assert_eq!(row(&t, 1), "");
}

#[test]
fn backspace_and_tabs() {
    let mut t = t(20, 3);
    feed(&mut t, "a\tb");
    assert_eq!(row(&t, 0), "a       b");
    feed(&mut t, "\r\x1b[2Kab\x08c");
    assert_eq!(row(&t, 0), "ac");
}

#[test]
fn tab_stops_set_and_clear() {
    let mut t = t(20, 3);
    feed(&mut t, "\x1b[3g\x1b[1;6H\x1bH\r\tX");
    assert_eq!(row(&t, 0), "     X");
}

#[test]
fn csi_cursor_addressing() {
    let mut t = t(20, 6);
    feed(&mut t, "\x1b[3;5Hx");
    assert_eq!(row(&t, 2), "    x");
    feed(&mut t, "\x1b[2Ay\x1b[3By");
    assert_eq!(row(&t, 0), "     y");
    assert_eq!(row(&t, 4), "     y");
    feed(&mut t, "\x1b[1;1H\x1b[99C\x1b[99B");
    assert_eq!(t.cursor_pos(), (19, 5));
    feed(&mut t, "\x1b[4G");
    assert_eq!(t.cursor_pos().0, 3);
    feed(&mut t, "\x1b[2d");
    assert_eq!(t.cursor_pos().1, 1);
    check_invariants(&t);
}

#[test]
fn erase_in_line_modes() {
    let mut a = t(10, 3);
    feed(&mut a, "0123456789\x1b[1;5H\x1b[K");
    assert_eq!(row(&a, 0), "0123");
    let mut b = t(10, 3);
    feed(&mut b, "0123456789\x1b[1;5H\x1b[1K");
    assert_eq!(row(&b, 0), "     56789");
    let mut c = t(10, 3);
    feed(&mut c, "0123456789\x1b[1;5H\x1b[2K");
    assert_eq!(row(&c, 0), "");
}

#[test]
fn erase_in_display_modes() {
    let mut a = t(6, 3);
    feed(&mut a, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2;3H\x1b[J");
    assert_eq!(row(&a, 0), "aaaaaa");
    assert_eq!(row(&a, 1), "bb");
    assert_eq!(row(&a, 2), "");
    let mut b = t(6, 3);
    feed(&mut b, "aaaaaa\r\nbbbbbb\r\ncccccc\x1b[2;3H\x1b[1J");
    assert_eq!(row(&b, 0), "");
    assert_eq!(row(&b, 1), "   bbb");
    assert_eq!(row(&b, 2), "cccccc");
    let mut c = t(6, 3);
    feed(&mut c, "aaaaaa\r\nbbbbbb\x1b[2J");
    assert_eq!(c.screen_text().trim(), "");
}

#[test]
fn insert_delete_erase_characters() {
    let mut a = t(10, 2);
    feed(&mut a, "abcdef\x1b[1;3H\x1b[2@");
    assert_eq!(row(&a, 0), "ab  cdef");
    let mut b = t(10, 2);
    feed(&mut b, "abcdef\x1b[1;3H\x1b[2P");
    assert_eq!(row(&b, 0), "abef");
    let mut c = t(10, 2);
    feed(&mut c, "abcdef\x1b[1;3H\x1b[2X");
    assert_eq!(row(&c, 0), "ab  ef");
    let mut d = t(10, 2);
    feed(&mut d, "abcdef\x1b[1;3H\x1b[4hXY");
    assert_eq!(row(&d, 0), "abXYcdef");
}

#[test]
fn scrolling_pushes_to_scrollback() {
    let mut t = t(10, 5);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5\r\nA");
    assert_eq!(t.scrollback_len(), 1);
    assert_eq!(t.row_string(t.combined_row(0)), "1");
    assert_eq!(row(&t, 0), "2");
    assert_eq!(row(&t, 4), "A");
    check_invariants(&t);
}

#[test]
fn scroll_region_does_not_touch_scrollback() {
    let mut t = t(10, 5);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5\x1b[2;4r\x1b[4;1H\n");
    assert_eq!(t.scrollback_len(), 0);
    assert_eq!(row(&t, 0), "1");
    assert_eq!(row(&t, 1), "3");
    assert_eq!(row(&t, 2), "4");
    assert_eq!(row(&t, 3), "");
    assert_eq!(row(&t, 4), "5");
}

#[test]
fn reverse_index_scrolls_down_at_top() {
    let mut t = t(10, 4);
    feed(&mut t, "a\r\nb\x1b[H\x1bM");
    assert_eq!(row(&t, 0), "");
    assert_eq!(row(&t, 1), "a");
    assert_eq!(row(&t, 2), "b");
}

#[test]
fn insert_and_delete_lines() {
    let mut t = t(10, 5);
    feed(&mut t, "1\r\n2\r\n3\x1b[2;1H\x1b[L");
    assert_eq!(row(&t, 1), "");
    assert_eq!(row(&t, 2), "2");
    assert_eq!(row(&t, 3), "3");
    feed(&mut t, "\x1b[M");
    assert_eq!(row(&t, 1), "2");
    assert_eq!(row(&t, 2), "3");
}

#[test]
fn origin_mode_is_relative_to_scroll_region() {
    let mut t = t(10, 6);
    feed(&mut t, "\x1b[2;4r\x1b[?6h\x1b[1;1Hx");
    assert_eq!(row(&t, 1), "x");
    feed(&mut t, "\x1b[?6l\x1b[1;1Hy");
    assert_eq!(row(&t, 0), "y");
}

#[test]
fn repeat_previous_character() {
    let mut t = t(10, 2);
    feed(&mut t, "a\x1b[3b");
    assert_eq!(row(&t, 0), "aaaa");
}

#[test]
fn save_restore_cursor_keeps_attributes() {
    let mut t = t(10, 3);
    feed(&mut t, "\x1b[1;31m\x1b7\x1b[0m\x1b[3;3H\x1b8x");
    let c = cell(&t, 0, 0);
    assert_eq!(c.ch, 'x');
    assert_eq!(c.fg, Color::Indexed(1));
    assert!(c.flags & attr::BOLD != 0);
}

// ---------------------------------------------------------------------------
// Colours and attributes
// ---------------------------------------------------------------------------

#[test]
fn sgr_basic_bright_and_reset() {
    let mut t = t(20, 2);
    feed(&mut t, "\x1b[1;31mA\x1b[0mB\x1b[91;102mC");
    let a = cell(&t, 0, 0);
    assert_eq!((a.ch, a.fg), ('A', Color::Indexed(1)));
    assert!(a.flags & attr::BOLD != 0);
    let b = cell(&t, 1, 0);
    assert_eq!((b.fg, b.flags), (Color::Default, 0));
    let c = cell(&t, 2, 0);
    assert_eq!((c.fg, c.bg), (Color::Indexed(9), Color::Indexed(10)));
}

#[test]
fn sgr_256_and_truecolor_in_both_syntaxes() {
    let mut t = t(20, 2);
    feed(&mut t, "\x1b[38;5;196mA\x1b[48;2;1;2;3mB\x1b[38:2::10:20:30mC\x1b[38:5:33mD\x1b[38:2:7:8:9mE");
    assert_eq!(cell(&t, 0, 0).fg, Color::Indexed(196));
    assert_eq!(cell(&t, 1, 0).bg, Color::Rgb(1, 2, 3));
    assert_eq!(cell(&t, 2, 0).fg, Color::Rgb(10, 20, 30));
    assert_eq!(cell(&t, 3, 0).fg, Color::Indexed(33));
    assert_eq!(cell(&t, 4, 0).fg, Color::Rgb(7, 8, 9));
}

#[test]
fn sgr_underline_styles_and_colour() {
    let mut t = t(20, 2);
    feed(&mut t, "\x1b[4:3mA\x1b[4mB\x1b[21mC\x1b[24mD\x1b[58;2;1;2;3m\x1b[4mE");
    assert!(cell(&t, 0, 0).flags & attr::CURLY_UL != 0);
    assert!(cell(&t, 1, 0).flags & attr::UNDERLINE != 0);
    assert!(cell(&t, 2, 0).flags & attr::DOUBLE_UL != 0);
    assert_eq!(cell(&t, 3, 0).flags & attr::ANY_UL, 0);
    assert_eq!(cell(&t, 4, 0).ul, Color::Rgb(1, 2, 3));
}

#[test]
fn erase_uses_background_colour() {
    let mut t = t(6, 2);
    feed(&mut t, "\x1b[44m\x1b[2K");
    let c = cell(&t, 3, 0);
    assert_eq!((c.ch, c.bg), (' ', Color::Indexed(4)));
}

#[test]
fn dec_special_graphics() {
    let mut t = t(10, 2);
    feed(&mut t, "\x1b(0lqk\x1b(Bx");
    assert_eq!(row(&t, 0), "┌─┐x");
}

// ---------------------------------------------------------------------------
// Alternate screen, modes
// ---------------------------------------------------------------------------

#[test]
fn alt_screen_1049_saves_and_restores() {
    let mut t = t(10, 3);
    feed(&mut t, "main\x1b[?1049h");
    assert!(t.in_alt_screen());
    assert_eq!(t.screen_text().trim(), "");
    feed(&mut t, "alt\x1b[?1049l");
    assert!(!t.in_alt_screen());
    assert_eq!(row(&t, 0), "main");
    assert_eq!(t.cursor_pos(), (4, 0));
    check_invariants(&t);
}

#[test]
fn alt_screen_has_no_scrollback() {
    let mut t = t(10, 3);
    feed(&mut t, "\x1b[?1049h");
    for i in 0..20 {
        feed(&mut t, &format!("line{i}\r\n"));
    }
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn mode_toggles_are_tracked_and_reported() {
    let mut t = t(10, 3);
    feed(&mut t, "\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?1h\x1b[?25l\x1b[?1004h");
    assert!(t.modes.bracketed_paste && t.modes.app_cursor && t.modes.focus_events);
    assert_eq!(t.modes.mouse, MouseMode::Normal);
    assert_eq!(t.modes.mouse_enc, MouseEnc::Sgr);
    assert!(!t.modes.cursor_visible);
    feed(&mut t, "\x1b[?1003h");
    assert_eq!(t.modes.mouse, MouseMode::Any);
    feed(&mut t, "\x1b[?1000l");
    assert_eq!(t.modes.mouse, MouseMode::Off);
    let _ = t.take_events();
    feed(&mut t, "\x1b[?2004$p\x1b[?9999$p");
    let r = replies(&mut t);
    assert_eq!(r[0], "\x1b[?2004;1$y");
    assert_eq!(r[1], "\x1b[?9999;0$y");
}

#[test]
fn cursor_style_decscusr() {
    let mut t = t(10, 3);
    feed(&mut t, "\x1b[5 q");
    assert_eq!(t.cursor_style(), CursorStyle { shape: CursorShape::Bar, blink: true });
    feed(&mut t, "\x1b[2 q");
    assert_eq!(t.cursor_style(), CursorStyle { shape: CursorShape::Block, blink: false });
    feed(&mut t, "\x1b[0 q");
    assert_eq!(t.cursor_style(), CursorStyle { shape: CursorShape::Block, blink: true });
}

#[test]
fn soft_and_hard_reset() {
    let mut t = t(10, 3);
    feed(&mut t, "\x1b[?7l\x1b[1;31mx\x1b[!p");
    assert!(t.modes.autowrap);
    assert_eq!(t.cursor.pen, Pen::default());
    feed(&mut t, "\x1bc");
    assert_eq!(t.screen_text().trim(), "");
    assert_eq!(t.cursor_pos(), (0, 0));
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[test]
fn device_attribute_and_status_replies() {
    let mut t = t(20, 10);
    feed(&mut t, "\x1b[c\x1b[5n\x1b[3;7H\x1b[6n\x1b[>q");
    let r = replies(&mut t);
    assert_eq!(r[0], "\x1b[?62;22c");
    assert_eq!(r[1], "\x1b[0n");
    assert_eq!(r[2], "\x1b[3;7R");
    assert!(r[3].starts_with("\x1bP>|mitos-terminal("));
}

#[test]
fn text_area_report_and_no_title_report() {
    let mut t = t(20, 10);
    feed(&mut t, "\x1b]0;secret\x07\x1b[18t\x1b[21t");
    let r = replies(&mut t);
    assert_eq!(r.len(), 1, "CSI 21 t (title report) must never be answered");
    assert_eq!(r[0], "\x1b[8;10;20t");
}

// ---------------------------------------------------------------------------
// OSC
// ---------------------------------------------------------------------------

#[test]
fn osc_title_and_length_cap() {
    let mut t = t(20, 3);
    feed(&mut t, "\x1b]0;My Title\x07");
    assert_eq!(t.title(), "My Title");
    feed(&mut t, "\x1b]2;Second\x1b\\");
    assert_eq!(t.title(), "Second");
    let long = "x".repeat(900);
    feed(&mut t, &format!("\x1b]0;{long}\x07"));
    assert!(t.title().chars().count() <= 256);
    t.set_policy(Policy { title: false, ..Policy::default() });
    feed(&mut t, "\x1b]0;nope\x07");
    assert_ne!(t.title(), "nope");
}

#[test]
fn title_stack() {
    let mut t = t(20, 3);
    feed(&mut t, "\x1b]0;one\x07\x1b[22;0t\x1b]0;two\x07");
    assert_eq!(t.title(), "two");
    feed(&mut t, "\x1b[23;0t");
    assert_eq!(t.title(), "one");
}

#[test]
fn osc8_hyperlinks() {
    let mut t = t(40, 3);
    feed(&mut t, "\x1b]8;;https://example.com/a;b\x07link\x1b]8;;\x07 plain");
    let c = cell(&t, 0, 0);
    assert_ne!(c.link, 0);
    assert_eq!(t.link_uri(c.link), Some("https://example.com/a;b"));
    assert_eq!(cell(&t, 3, 0).link, c.link);
    assert_eq!(cell(&t, 5, 0).link, 0);
    let line = t.view_abs(0);
    let span = t.url_at(line, 2).unwrap();
    assert_eq!((span.c0, span.c1), (0, 3));
    assert_eq!(span.url, "https://example.com/a;b");
}

#[test]
fn osc8_respects_policy_and_limits() {
    let mut t = t(40, 3);
    t.set_policy(Policy { hyperlinks: false, ..Policy::default() });
    feed(&mut t, "\x1b]8;;https://x.org\x07hi");
    assert_eq!(cell(&t, 0, 0).link, 0);
    let mut u = t2();
    u.set_policy(Policy { max_uri: 10, ..Policy::default() });
    feed(&mut u, "\x1b]8;;https://example.com/very/long\x07hi");
    assert_eq!(cell(&u, 0, 0).link, 0);
}

#[test]
fn osc7_cwd_local_only() {
    let mut t = t(20, 3);
    feed(&mut t, "\x1b]7;file:///home/user/My%20Dir\x07");
    assert_eq!(t.cwd(), Some("/home/user/My Dir"));
    let mut u = t2();
    feed(&mut u, "\x1b]7;file://some-remote-host.invalid/tmp\x07");
    assert_eq!(u.cwd(), None);
}

#[test]
fn osc52_write_allowed_read_denied() {
    let mut t = t(20, 3);
    feed(&mut t, "\x1b]52;c;aGVsbG8=\x07");
    let ev = t.take_events();
    assert!(matches!(&ev[0], TermEvent::ClipboardStore { text } if text == "hello"));
    feed(&mut t, "\x1b]52;c;?\x07");
    assert!(t.take_events().is_empty(), "clipboard reads must never be answered");
    t.set_policy(Policy { clipboard_write: false, ..Policy::default() });
    feed(&mut t, "\x1b]52;c;aGVsbG8=\x07");
    assert!(t.take_events().is_empty());
}

#[test]
fn osc4_and_dynamic_colours() {
    let mut t = t(20, 3);
    t.set_theme_colors([200, 200, 200], [4, 10, 18], [1, 2, 3], crate::theme::Theme::dark().ansi);
    feed(&mut t, "\x1b]4;1;rgb:ff/00/00\x07");
    assert_eq!(t.overrides.palette[1], Some([255, 0, 0]));
    let _ = t.take_events();
    feed(&mut t, "\x1b]4;1;?\x07\x1b]11;?\x1b\\");
    let r = replies(&mut t);
    assert_eq!(r[0], "\x1b]4;1;rgb:ffff/0000/0000\x07");
    assert_eq!(r[1], "\x1b]11;rgb:0404/0a0a/1212\x1b\\");
    feed(&mut t, "\x1b]104\x07");
    assert_eq!(t.overrides.palette[1], None);
    feed(&mut t, "\x1b]10;#00ff00\x07");
    assert_eq!(t.overrides.fg, Some([0, 255, 0]));
    feed(&mut t, "\x1b]110\x07");
    assert_eq!(t.overrides.fg, None);
}

#[test]
fn notifications_and_progress() {
    let mut t = t(20, 3);
    feed(&mut t, "\x1b]777;notify;Build;done\x07\x1b]9;4;1;42\x07\x1b]9;hello\x07");
    let ev = t.take_events();
    assert!(ev.iter().any(|e| matches!(e, TermEvent::Notify { title, body } if title == "Build" && body == "done")));
    assert!(ev.iter().any(|e| matches!(e, TermEvent::Progress { state: 1, percent: 42 })));
    assert!(ev.iter().any(|e| matches!(e, TermEvent::Notify { body, .. } if body == "hello")));
    t.set_policy(Policy { notifications: false, ..Policy::default() });
    feed(&mut t, "\x1b]777;notify;a;b\x07");
    assert!(t.take_events().is_empty());
}

#[test]
fn bell_event() {
    let mut t = t(20, 3);
    feed(&mut t, "\x07");
    assert!(matches!(t.take_events()[0], TermEvent::Bell));
}

#[test]
fn osc133_tracks_commands_and_prompts() {
    let mut t = t(40, 6);
    feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07ls -la\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07");
    let ev = t.take_events();
    assert!(ev.iter().any(|e| matches!(e, TermEvent::CommandStarted { command } if command == "ls -la")));
    assert!(ev.iter().any(|e| matches!(e, TermEvent::CommandFinished { exit: Some(0), command, .. } if command == "ls -la")));
    assert_eq!(t.commands().len(), 1);
    assert_eq!(t.commands()[0].exit, Some(0));
    assert_eq!(t.prompt_lines(), vec![t.screen_top_abs()]);
    assert_eq!(t.jump_prompt(t.screen_top_abs() + 3, true), Some(t.screen_top_abs()));
}

#[test]
fn command_not_found_scan_is_armed_only_after_enter() {
    let mut t = t(60, 5);
    feed(&mut t, "bash: nope: command not found\r\n");
    assert!(t.take_events().is_empty());
    t.arm_command_scan();
    feed(&mut t, "bash: foo: command not found\r\n");
    let ev = t.take_events();
    assert!(ev.iter().any(|e| matches!(e, TermEvent::MissingCommand(c) if c == "foo")));
    t.arm_command_scan();
    feed(&mut t, "bash: foo: command not found\r\n");
    assert!(t.take_events().is_empty(), "same command is suggested once");
}

#[test]
fn mitos_execution_blocks() {
    let mut t = t(40, 6);
    let osc = mitos_utils::ipc::OSC_NEW_BLOCK;
    feed(&mut t, &format!("\x1b]{osc};user@host:~$ \x07"));
    assert_eq!(row(&t, 0), "user@host:~$");
    assert!(t.view_row(0).marks & mark::BLOCK != 0);
    assert_eq!(t.cursor_pos(), (0, 1));
    feed(&mut t, &format!("out\r\n\x1b]{osc};next$ \x07"));
    let ev = t.take_events();
    assert!(ev.iter().any(|e| matches!(e, TermEvent::BlockClosed { .. })));
    assert_eq!(t.block_prompt(), "next$");
}

#[test]
fn widgets_are_anchored_and_policy_gated() {
    let mut t = t(40, 6);
    let w = || RichWidget::Button { label: "Go".to_string(), cmd: "true".to_string() };
    assert!(t.inject_widget(w(), true));
    assert_eq!(t.widgets().len(), 1);
    assert_eq!(t.widgets()[0].line, t.screen_top_abs());
    assert!(t.view_row(0).marks & mark::WIDGET != 0);
    assert_eq!(t.cursor_pos().1, 1);
    t.set_policy(Policy { widgets: false, ..Policy::default() });
    assert!(!t.inject_widget(w(), false));
    assert!(t.inject_widget(w(), true), "trusted widgets ignore the child-facing policy");
}

#[test]
fn runaway_osc_is_cancelled() {
    let mut t = t(20, 3);
    let junk = "a".repeat(300_000);
    feed(&mut t, &format!("\x1b]0;{junk}\x07hi"));
    assert_eq!(row(&t, 0), "hi");
    check_invariants(&t);
}

// ---------------------------------------------------------------------------
// Unicode
// ---------------------------------------------------------------------------

#[test]
fn char_widths() {
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('é'), 1);
    assert_eq!(char_width('世'), 2);
    assert_eq!(char_width('１'), 2);
    assert_eq!(char_width('ｱ'), 1);
    assert_eq!(char_width('😀'), 2);
    assert_eq!(char_width('\u{301}'), 0);
    assert_eq!(char_width('\u{200d}'), 0);
    assert_eq!(char_width('\u{fe0f}'), 0);
    assert_eq!(char_width('\u{7}'), 0);
}

#[test]
fn wide_characters_occupy_two_cells() {
    let mut t = t(10, 3);
    feed(&mut t, "a世b");
    assert_eq!(row(&t, 0), "a世b");
    assert!(cell(&t, 1, 0).flags & attr::WIDE != 0);
    assert!(cell(&t, 2, 0).flags & attr::WIDE_TAIL != 0);
    assert_eq!(cell(&t, 3, 0).ch, 'b');
    assert_eq!(t.cursor_pos(), (4, 0));
    check_invariants(&t);
}

#[test]
fn wide_character_wraps_instead_of_splitting() {
    let mut t = t(4, 3);
    feed(&mut t, "abc世");
    assert_eq!(row(&t, 0), "abc");
    assert_eq!(row(&t, 1), "世");
    check_invariants(&t);
}

#[test]
fn overwriting_half_of_a_wide_char_blanks_the_other_half() {
    let mut t = t(10, 3);
    feed(&mut t, "世界\x1b[1;2Hx");
    check_invariants(&t);
    assert_eq!(cell(&t, 0, 0).ch, ' ');
    assert_eq!(cell(&t, 1, 0).ch, 'x');
}

#[test]
fn combining_marks_join_the_previous_cell() {
    let mut t = t(10, 3);
    feed(&mut t, "e\u{301}x");
    let c = cell(&t, 0, 0);
    assert_eq!(t.cluster_str(c.ch), Some("e\u{301}"));
    assert_eq!(row(&t, 0), "e\u{301}x");
    assert_eq!(t.cursor_pos(), (2, 0));
}

#[test]
fn invalid_utf8_becomes_replacement_character() {
    let mut t = t(10, 3);
    t.process(&[b'a', 0xff, b'b']);
    let text = row(&t, 0);
    assert!(text.starts_with('a') && text.contains('\u{fffd}') && text.ends_with('b'), "{text:?}");
}

#[test]
fn utf8_split_across_chunks() {
    let mut t = t(10, 3);
    let bytes = "世".as_bytes();
    t.process(&bytes[..1]);
    t.process(&bytes[1..2]);
    t.process(&bytes[2..]);
    assert_eq!(row(&t, 0), "世");
}

// ---------------------------------------------------------------------------
// Scrollback, resize
// ---------------------------------------------------------------------------

#[test]
fn scrollback_is_capped_and_abs_ids_stay_stable() {
    let mut t = Term::new(10, 3, 5);
    for i in 0..20 {
        feed(&mut t, &format!("line{i}\r\n"));
    }
    assert_eq!(t.scrollback_len(), 5);
    assert_eq!(t.dropped(), 13);
    assert_eq!(t.row_string(t.row_by_abs(13).unwrap()), "line13");
    assert!(t.row_by_abs(12).is_none());
    assert_eq!(row(&t, 0), "line18");
    check_invariants(&t);
}

#[test]
fn view_offset_holds_position_while_output_streams() {
    let mut t = t(10, 3);
    for i in 0..10 {
        feed(&mut t, &format!("l{i}\r\n"));
    }
    assert!(t.scroll_display(2));
    assert_eq!(t.display_offset, 2);
    let before = t.row_string(t.view_row(0));
    feed(&mut t, "x\r\n");
    assert_eq!(t.display_offset, 3);
    assert_eq!(t.row_string(t.view_row(0)), before);
    t.scroll_to_bottom();
    assert_eq!(t.display_offset, 0);
}

#[test]
fn resize_reflows_wrapped_lines_and_tracks_cursor() {
    let mut t = t(10, 5);
    feed(&mut t, "0123456789abcdef");
    assert_eq!((row(&t, 0).as_str(), row(&t, 1).as_str()), ("0123456789", "abcdef"));
    t.resize(20, 5);
    assert_eq!(row(&t, 0), "0123456789abcdef");
    assert_eq!(t.cursor_pos(), (16, 0));
    t.resize(8, 5);
    assert_eq!(row(&t, 0), "01234567");
    assert_eq!(row(&t, 1), "89abcdef");
    assert_eq!(t.cursor_pos(), (0, 2));
    check_invariants(&t);
}

#[test]
fn resize_height_moves_rows_through_scrollback() {
    let mut t = t(10, 5);
    feed(&mut t, "1\r\n2\r\n3\r\n4\r\n5");
    t.resize(10, 3);
    assert_eq!(t.scrollback_len(), 2);
    assert_eq!(row(&t, 0), "3");
    assert_eq!(row(&t, 2), "5");
    assert_eq!(t.cursor_pos().1, 2);
    t.resize(10, 5);
    assert_eq!(t.scrollback_len(), 0);
    assert_eq!(row(&t, 0), "1");
    assert_eq!(row(&t, 4), "5");
    check_invariants(&t);
}

#[test]
fn resize_in_alt_screen_preserves_primary() {
    let mut t = t(10, 4);
    feed(&mut t, "primary\x1b[?1049hALT");
    t.resize(6, 3);
    check_invariants(&t);
    feed(&mut t, "\x1b[?1049l");
    check_invariants(&t);
    assert_eq!(t.row_string(t.combined_row(0)), "primar");
    assert_eq!(t.row_string(t.combined_row(1)), "y");
}

#[test]
fn resize_never_panics_on_extremes() {
    let mut t = t(80, 24);
    feed(&mut t, "世界 hello\r\n世界 world\r\n");
    for (c, r) in [(1, 1), (2, 1), (1, 5), (500, 3), (3, 500), (80, 24), (7, 2)] {
        t.resize(c, r);
        check_invariants(&t);
    }
}

// ---------------------------------------------------------------------------
// Selection, search, URLs
// ---------------------------------------------------------------------------

#[test]
fn selection_modes() {
    let mut t = t(20, 4);
    feed(&mut t, "hello world\r\nsecond line");
    t.sel_begin(SelMode::Simple, t.view_point(0, 0));
    t.sel_update(t.view_point(0, 4));
    assert_eq!(t.selection_text().as_deref(), Some("hello"));
    t.sel_begin(SelMode::Word, t.view_point(0, 7));
    assert_eq!(t.selection_text().as_deref(), Some("world"));
    t.sel_begin(SelMode::Line, t.view_point(1, 3));
    assert_eq!(t.selection_text().as_deref(), Some("second line"));
    t.sel_begin(SelMode::Simple, t.view_point(0, 6));
    t.sel_update(t.view_point(1, 5));
    assert_eq!(t.selection_text().as_deref(), Some("world\nsecond"));
    t.sel_clear();
    assert!(t.selection_text().is_none());
}

#[test]
fn block_selection() {
    let mut t = t(8, 4);
    feed(&mut t, "abcd\r\nefgh\r\nijkl");
    t.sel_begin(SelMode::Block, t.view_point(0, 1));
    t.sel_update(t.view_point(2, 2));
    assert_eq!(t.selection_text().as_deref(), Some("bc\nfg\njk"));
}

#[test]
fn soft_wrapped_lines_copy_without_newlines() {
    let mut t = t(5, 5);
    feed(&mut t, "hello world");
    t.sel_begin(SelMode::Line, t.view_point(1, 0));
    assert_eq!(t.selection_text().as_deref(), Some("hello world"));
}

#[test]
fn selection_survives_scrolling_and_dies_with_its_lines() {
    let mut t = Term::new(10, 3, 4);
    feed(&mut t, "keep\r\n");
    t.sel_begin(SelMode::Line, t.view_point(0, 0));
    for i in 0..3 {
        feed(&mut t, &format!("x{i}\r\n"));
    }
    assert_eq!(t.selection_text().as_deref(), Some("keep"));
    for i in 0..20 {
        feed(&mut t, &format!("y{i}\r\n"));
    }
    assert!(t.selection.is_none(), "selection must be dropped once its lines leave the scrollback");
}

#[test]
fn search_is_case_aware_and_crosses_soft_wraps() {
    let mut t = t(5, 6);
    feed(&mut t, "Hello World");
    assert!(t.search("hello", true).is_empty());
    assert_eq!(t.search("hello", false).len(), 1);
    let hits = t.search("lo wo", false);
    assert_eq!(hits.len(), 1);
    let top = t.screen_top_abs();
    assert_eq!(hits[0].spans, vec![(top, 3, 4), (top + 1, 0, 2)]);
    assert!(t.search("", false).is_empty());
}

#[test]
fn plain_text_url_detection() {
    let mut t = t(60, 3);
    feed(&mut t, "see https://example.com/a_(b). ok (https://x.org) mailto:a@b.co");
    let row0 = t.view_row(0).clone();
    let urls = t.urls_in_row(&row0);
    let list: Vec<&str> = urls.iter().map(|u| u.url.as_str()).collect();
    assert_eq!(list, vec!["https://example.com/a_(b)", "https://x.org", "mailto:a@b.co"]);
    let line = t.view_abs(0);
    assert_eq!(t.url_at(line, 8).map(|u| u.url), Some("https://example.com/a_(b)".to_string()));
    assert!(t.url_at(line, 0).is_none());
}

// ---------------------------------------------------------------------------
// Damage tracking, ghost text, fx
// ---------------------------------------------------------------------------

#[test]
fn damage_is_reported_once() {
    let mut t = t(10, 3);
    assert_eq!(t.take_damage(), Damage::Full);
    assert_eq!(t.take_damage(), Damage::None);
    feed(&mut t, "a");
    assert_eq!(t.take_damage(), Damage::Rows(vec![0]));
    let seq = t.seqno();
    feed(&mut t, "\r\n\r\n\r\n\r\n");
    assert!(t.seqno() > seq);
    assert_eq!(t.take_damage(), Damage::Full);
}

#[test]
fn ghost_text_shrinks_as_you_type_and_clears_on_mismatch() {
    let mut t = t(20, 3);
    t.set_ghost_text(Some("cd".to_string()));
    feed(&mut t, "c");
    assert_eq!(t.ghost_text(), Some("d"));
    feed(&mut t, "x");
    assert_eq!(t.ghost_text(), None);
}

#[test]
fn fresh_spans_are_coalesced() {
    let mut t = t(20, 3);
    feed(&mut t, "hello");
    assert_eq!(t.fresh().len(), 1);
    let f = t.fresh()[0];
    assert_eq!((f.c0, f.c1, f.error), (0, 5, false));
}

// ---------------------------------------------------------------------------
// Stress / fuzz — must never panic or break invariants
// ---------------------------------------------------------------------------

struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

#[test]
fn random_bytes_and_escape_soup_never_break_invariants() {
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    let alphabet: &[u8] = b"\x1b[]();?0123456789:;<>=!\"'#$ %&*+,-./@ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz\x07\x08\x09\x0a\x0d\x0e\x0f\x18\x1a";
    let mut t = Term::new(37, 11, 200);
    for round in 0..400 {
        let mut chunk = Vec::with_capacity(4096);
        for _ in 0..4096 {
            let r = rng.next();
            let b = if r % 9 == 0 { (r >> 8) as u8 } else { alphabet[(r >> 16) as usize % alphabet.len()] };
            chunk.push(b);
        }
        t.process(&chunk);
        let _ = t.take_events();
        if round % 37 == 0 {
            let c = 1 + (rng.next() % 90) as usize;
            let r = 1 + (rng.next() % 30) as usize;
            t.resize(c, r);
        }
        check_invariants(&t);
    }
}

#[test]
fn huge_parameters_are_harmless() {
    let mut t = t(20, 5);
    feed(&mut t, "\x1b[999999999@\x1b[999999999P\x1b[999999999L\x1b[999999999M\x1b[999999999S\x1b[999999999T");
    feed(&mut t, "\x1b[999999999;999999999H\x1b[999999999b\x1b[999999999X\x1b[65535C");
    check_invariants(&t);
}

#[test]
fn flood_of_output_stays_bounded() {
    let mut t = Term::new(80, 24, 1000);
    let line = format!("{}\r\n", "x".repeat(79));
    let block = line.repeat(1000);
    for _ in 0..50 {
        t.process(block.as_bytes());
    }
    assert_eq!(t.scrollback_len(), 1000);
    check_invariants(&t);
}
