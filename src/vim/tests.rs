use super::*;

fn engine(text: &str) -> VimEngine {
    VimEngine::new(text.to_string())
}

fn press(e: &mut VimEngine, events: &[VimEvent]) {
    for ev in events {
        e.apply(ev.clone());
    }
}

fn ch(c: char) -> VimEvent {
    VimEvent::Char(c)
}

#[test]
fn starts_in_normal_mode() {
    let e = engine("hello");
    assert_eq!(e.mode(), Mode::Normal);
    assert_eq!(e.cursor(), 0);
}

#[test]
fn h_moves_cursor_left_within_line() {
    let mut e = engine("hello");
    e.cursor = 3;
    e.apply(ch('h'));
    assert_eq!(e.cursor(), 2);
}

#[test]
fn h_stops_at_line_start() {
    let mut e = engine("ab\ncd");
    e.cursor = 3; // 'c'
    e.apply(ch('h'));
    assert_eq!(e.cursor(), 3, "h must not cross newline");
}

#[test]
fn l_moves_cursor_right() {
    let mut e = engine("hello");
    e.apply(ch('l'));
    assert_eq!(e.cursor(), 1);
}

#[test]
fn l_stops_at_line_end() {
    let mut e = engine("ab\ncd");
    e.cursor = 1; // 'b'
    e.apply(ch('l'));
    // Vim's `l` stops one before the newline; in our minimal model
    // we already are on the last column of the line.
    assert_eq!(
        e.cursor(),
        2,
        "l from 'b' lands on the newline position (stops before crossing)"
    );
    e.apply(ch('l'));
    assert_eq!(e.cursor(), 2, "l must not cross newline");
}

#[test]
fn j_moves_down_keeping_column() {
    let mut e = engine("abc\ndef");
    e.cursor = 1; // 'b'
    e.apply(ch('j'));
    assert_eq!(e.cursor(), 5, "moved to 'e' (column 1 of line 2)");
}

#[test]
fn j_clamps_column_on_shorter_line() {
    let mut e = engine("abcdef\nxy");
    e.cursor = 4; // 'e' (column 4)
    e.apply(ch('j'));
    assert_eq!(e.cursor(), 9, "shorter line clamps to its end");
}

#[test]
fn k_moves_up() {
    let mut e = engine("abc\ndef");
    e.cursor = 5; // 'e'
    e.apply(ch('k'));
    assert_eq!(e.cursor(), 1);
}

#[test]
fn zero_jumps_to_line_start() {
    let mut e = engine("abc\ndef");
    e.cursor = 6; // 'f'
    e.apply(ch('0'));
    assert_eq!(e.cursor(), 4);
}

#[test]
fn dollar_jumps_to_line_end() {
    let mut e = engine("abc\ndef");
    e.cursor = 4; // 'd'
    e.apply(ch('$'));
    assert_eq!(
        e.cursor(),
        7,
        "ends at offset of '\\n' — equivalent to last char in our model"
    );
}

#[test]
fn gg_jumps_to_buffer_start() {
    let mut e = engine("abc\ndef");
    e.cursor = 5;
    press(&mut e, &[ch('g'), ch('g')]);
    assert_eq!(e.cursor(), 0);
}

#[test]
fn g_capital_jumps_to_last_line() {
    let mut e = engine("abc\ndef\nghi");
    e.cursor = 0;
    e.apply(ch('G'));
    assert_eq!(e.cursor(), 8, "first char of last line");
}

#[test]
fn i_enters_insert_mode() {
    let mut e = engine("abc");
    e.apply(ch('i'));
    assert_eq!(e.mode(), Mode::Insert);
}

#[test]
fn esc_returns_to_normal_from_insert() {
    let mut e = engine("abc");
    e.apply(ch('i'));
    e.cursor = 2;
    e.apply(VimEvent::Escape);
    assert_eq!(e.mode(), Mode::Normal);
    assert_eq!(e.cursor(), 1, "esc steps cursor back one");
}

#[test]
fn insert_char_appends() {
    let mut e = engine("ab");
    e.cursor = 2;
    e.apply(ch('i'));
    e.apply(ch('c'));
    assert_eq!(e.buffer(), "abc");
    assert_eq!(e.cursor(), 3);
}

#[test]
fn insert_at_start() {
    let mut e = engine("bc");
    e.apply(ch('i'));
    e.apply(ch('a'));
    assert_eq!(e.buffer(), "abc");
}

#[test]
fn insert_backspace_deletes_previous_char() {
    let mut e = engine("abc");
    e.cursor = 3;
    e.apply(ch('i'));
    e.apply(VimEvent::Backspace);
    assert_eq!(e.buffer(), "ab");
    assert_eq!(e.cursor(), 2);
}

#[test]
fn x_deletes_char_under_cursor() {
    let mut e = engine("abc");
    e.cursor = 1; // 'b'
    e.apply(ch('x'));
    assert_eq!(e.buffer(), "ac");
    assert_eq!(e.cursor(), 1, "cursor sits on what 'c' became");
}

#[test]
fn x_at_line_end_steps_cursor_back() {
    let mut e = engine("abc\ndef");
    e.cursor = 2; // 'c'
    e.apply(ch('x'));
    assert_eq!(e.buffer(), "ab\ndef");
    assert_eq!(e.cursor(), 1, "cursor falls back to 'b'");
}

#[test]
fn x_does_not_eat_newline() {
    let mut e = engine("abc\ndef");
    e.cursor = 3; // '\n'
    e.apply(ch('x'));
    assert_eq!(e.buffer(), "abc\ndef");
}

#[test]
fn dd_deletes_current_line() {
    let mut e = engine("abc\ndef\nghi");
    e.cursor = 5; // 'e'
    press(&mut e, &[ch('d'), ch('d')]);
    assert_eq!(e.buffer(), "abc\nghi");
}

#[test]
fn dd_deletes_last_line() {
    let mut e = engine("abc\ndef");
    e.cursor = 5;
    press(&mut e, &[ch('d'), ch('d')]);
    assert_eq!(e.buffer(), "abc\n");
}

#[test]
fn yy_then_p_duplicates_line() {
    let mut e = engine("abc\ndef");
    e.cursor = 0;
    press(&mut e, &[ch('y'), ch('y')]);
    e.apply(ch('p'));
    assert_eq!(e.buffer(), "abc\nabc\ndef");
}

#[test]
fn yy_at_buffer_end_then_p_appends_newline() {
    let mut e = engine("abc");
    e.cursor = 0;
    press(&mut e, &[ch('y'), ch('y')]);
    e.apply(ch('p'));
    assert_eq!(e.buffer(), "abc\nabc\n");
}

#[test]
fn x_yank_then_p_pastes_charwise() {
    let mut e = engine("abc");
    e.cursor = 0;
    e.apply(ch('x')); // yank 'a', buffer = "bc"
    e.cursor = 1; // 'c'
    e.apply(ch('p'));
    assert_eq!(e.buffer(), "bca", "p inserts the yanked char after cursor");
}

#[test]
fn u_undoes_last_edit() {
    let mut e = engine("abc");
    e.cursor = 1;
    e.apply(ch('x'));
    assert_eq!(e.buffer(), "ac");
    e.apply(ch('u'));
    assert_eq!(e.buffer(), "abc");
}

#[test]
fn ctrl_r_redoes_after_undo() {
    let mut e = engine("abc");
    e.cursor = 1;
    e.apply(ch('x'));
    e.apply(ch('u'));
    e.apply(VimEvent::CtrlR);
    assert_eq!(e.buffer(), "ac");
}

#[test]
fn fresh_edit_clears_redo_stack() {
    let mut e = engine("abc");
    e.cursor = 1;
    e.apply(ch('x')); // delete 'b'
    e.apply(ch('u')); // undo
    e.apply(ch('x')); // new edit clears redo
    e.apply(VimEvent::CtrlR);
    assert_eq!(e.buffer(), "ac", "redo should be a no-op now");
}

#[test]
fn o_opens_line_below_and_enters_insert() {
    let mut e = engine("abc\ndef");
    e.cursor = 1;
    e.apply(ch('o'));
    assert_eq!(e.buffer(), "abc\n\ndef");
    assert_eq!(e.mode(), Mode::Insert);
}

#[test]
fn w_jumps_to_next_word_start() {
    let mut e = engine("foo bar baz");
    e.cursor = 0;
    e.apply(ch('w'));
    assert_eq!(e.cursor(), 4);
}

#[test]
fn b_jumps_to_previous_word_start() {
    let mut e = engine("foo bar baz");
    e.cursor = 8; // 'b' of "baz"
    e.apply(ch('b'));
    assert_eq!(e.cursor(), 4);
}

#[test]
fn e_jumps_to_word_end() {
    let mut e = engine("foo bar");
    e.cursor = 0;
    e.apply(ch('e'));
    assert_eq!(e.cursor(), 2, "end of 'foo'");
}

#[test]
fn v_enters_visual() {
    let mut e = engine("abc");
    e.apply(ch('v'));
    assert_eq!(e.mode(), Mode::Visual);
    // Inclusive: anchor==cursor==0 still selects the char at 0.
    assert_eq!(e.visual_range(), Some(0..1));
}

#[test]
fn visual_yank_then_p() {
    let mut e = engine("abcdef");
    e.cursor = 1;
    e.apply(ch('v'));
    e.apply(ch('l'));
    e.apply(ch('l')); // selection covers "bcd"
    e.apply(ch('y'));
    assert_eq!(e.mode(), Mode::Normal);
    e.cursor = 5; // 'f'
    e.apply(ch('p'));
    assert_eq!(e.buffer(), "abcdefbcd", "charwise p inserts after cursor");
}

#[test]
fn visual_d_deletes_selection() {
    let mut e = engine("abcdef");
    e.cursor = 1;
    e.apply(ch('v'));
    e.apply(ch('l'));
    e.apply(ch('l'));
    e.apply(ch('d'));
    assert_eq!(e.buffer(), "aef");
    assert_eq!(e.mode(), Mode::Normal);
}

#[test]
fn replace_buffer_resets_state() {
    let mut e = engine("hello");
    e.cursor = 3;
    e.apply(ch('i'));
    e.replace_buffer("new content".to_string());
    assert_eq!(e.buffer(), "new content");
    assert_eq!(e.cursor(), 0);
    assert_eq!(e.mode(), Mode::Normal);
}

#[test]
fn handles_multibyte_chars_without_panic() {
    let mut e = engine("あいう");
    e.cursor = 0;
    e.apply(ch('l'));
    assert_eq!(e.cursor(), 3, "moved past 'あ' (3 bytes)");
    e.apply(ch('l'));
    assert_eq!(e.cursor(), 6);
}

#[test]
fn insert_japanese_after_normal_mode() {
    let mut e = engine("");
    e.apply(ch('i'));
    e.apply(ch('あ'));
    e.apply(ch('い'));
    assert_eq!(e.buffer(), "あい");
    assert_eq!(e.cursor(), 6);
}

#[test]
fn pending_prefix_clears_on_non_match() {
    let mut e = engine("abc\ndef");
    e.apply(ch('d')); // pending = D
    e.apply(ch('l')); // not 'd' → cancels pending, moves right
    assert_eq!(e.cursor(), 1);
    assert_eq!(e.buffer(), "abc\ndef");
}

#[test]
fn line_bounds_on_empty_line() {
    let s = "a\n\nb";
    assert_eq!(line_bounds(s, 2), (2, 2));
}

#[test]
fn line_bounds_on_last_line_no_trailing_newline() {
    let s = "a\nb";
    assert_eq!(line_bounds(s, 2), (2, 3));
}
