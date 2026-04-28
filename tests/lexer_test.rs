//! Lexer integration tests.
//!
//! These exercise the public `Lexer::tokenize` API end-to-end on small
//! source fragments rather than poking at internal helper functions. The
//! goal is to lock in observable behaviour so future refactors (e.g.
//! interning identifiers, switching to a streaming token API) can't
//! silently change what downstream phases see.
//!
//! Coverage groups:
//!   - delimiters, operators, prefixes
//!   - identifier classification (Triggers vs Keywords vs plain Identifiers)
//!   - numeric literals (integers, hex, long suffix, negative-as-binary)
//!   - coord literals vs trailing-identifier disambiguation
//!   - `%` GameVar prefix vs modulo operator
//!   - string and char literals, including `<…>` interpolation with
//!     embedded quotes
//!   - single- and multi-line comments + unterminated-comment error
//!   - line/column tracking
//!   - EOF token presence
//!   - error paths for unrecognised chars

use rs_compiler::lexer::Lexer;
use rs_compiler::token::{Kind, Token};
use std::path::PathBuf;

fn lex(src: &str) -> Vec<Token> {
    let path = PathBuf::from("test.rs2");
    Lexer::new(src, &path)
        .tokenize()
        .expect("lex failed unexpectedly")
}

fn lex_err(src: &str) -> String {
    let path = PathBuf::from("test.rs2");
    let err = Lexer::new(src, &path)
        .tokenize()
        .expect_err("expected lex error");
    err.message
}

/// Token kinds only — convenience for sequence assertions.
fn kinds(tokens: &[Token]) -> Vec<Kind> {
    tokens.iter().map(|t| t.kind.clone()).collect()
}

// ── Smoke / EOF ─────────────────────────────────────────────────────

#[test]
fn empty_input_yields_only_eof() {
    let toks = lex("");
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].kind, Kind::EndOfFile);
}

#[test]
fn whitespace_only_yields_only_eof() {
    let toks = lex("   \t\n  \r\n  ");
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].kind, Kind::EndOfFile);
}

#[test]
fn eof_is_always_last_token() {
    let toks = lex("[proc,foo]");
    assert_eq!(toks.last().unwrap().kind, Kind::EndOfFile);
}

// ── Delimiters and operators ────────────────────────────────────────

#[test]
fn all_single_char_delimiters() {
    let toks = lex("[](){};,:&|!#.~@");
    let expected = vec![
        Kind::LBracket,
        Kind::RBracket,
        Kind::LParen,
        Kind::RParen,
        Kind::LBrace,
        Kind::RBrace,
        Kind::Semicolon,
        Kind::Comma,
        Kind::Colon,
        Kind::LogicalAnd,
        Kind::LogicalOr,
        Kind::Not,
        Kind::Hash,
        Kind::Dot,
        Kind::ScriptCall,
        Kind::JumpCall,
        Kind::EndOfFile,
    ];
    assert_eq!(kinds(&toks), expected);
}

#[test]
fn comparison_operators_with_and_without_equals() {
    let toks = lex("< <= > >= =");
    let expected = vec![
        Kind::ComparisonOperator,
        Kind::ComparisonOperator,
        Kind::ComparisonOperator,
        Kind::ComparisonOperator,
        Kind::Equals,
        Kind::EndOfFile,
    ];
    assert_eq!(kinds(&toks), expected);
    // Spot-check the lexed text matches.
    assert_eq!(toks[0].value, "<");
    assert_eq!(toks[1].value, "<=");
    assert_eq!(toks[2].value, ">");
    assert_eq!(toks[3].value, ">=");
}

#[test]
fn arithmetic_operators() {
    let toks = lex("+ - * / %");
    // %  with no following identifier-start char is BinaryOperator.
    assert_eq!(
        kinds(&toks),
        vec![
            Kind::BinaryOperator, // +
            Kind::BinaryOperator, // -
            Kind::BinaryOperator, // *
            Kind::BinaryOperator, // /
            Kind::BinaryOperator, // %
            Kind::EndOfFile,
        ]
    );
}

// ── % disambiguation: GameVar vs modulo ─────────────────────────────

#[test]
fn percent_followed_by_identifier_is_gamevar() {
    let toks = lex("%my_varp");
    assert_eq!(
        kinds(&toks),
        vec![Kind::GameVar, Kind::Identifier, Kind::EndOfFile]
    );
    assert_eq!(toks[0].value, "%");
    assert_eq!(toks[1].value, "my_varp");
}

#[test]
fn percent_followed_by_digit_is_modulo() {
    let toks = lex("10 % 3");
    assert_eq!(
        kinds(&toks),
        vec![
            Kind::Number,
            Kind::BinaryOperator,
            Kind::Number,
            Kind::EndOfFile,
        ]
    );
}

#[test]
fn percent_followed_by_underscore_is_gamevar() {
    let toks = lex("%_underscored");
    assert_eq!(toks[0].kind, Kind::GameVar);
}

// ── Identifier classification ───────────────────────────────────────

#[test]
fn keywords_classify_correctly() {
    let toks = lex("if else while return calc null true false case default");
    assert_eq!(
        kinds(&toks),
        vec![
            Kind::If,
            Kind::Else,
            Kind::While,
            Kind::Return,
            Kind::Calc,
            Kind::Null,
            Kind::BooleanTrue,
            Kind::BooleanFalse,
            Kind::Case,
            Kind::Default,
            Kind::EndOfFile,
        ]
    );
}

#[test]
fn switch_prefix_classifies_as_switch() {
    let toks = lex("switch_int switch_string switch_obj");
    assert_eq!(toks[0].kind, Kind::Switch);
    assert_eq!(toks[1].kind, Kind::Switch);
    assert_eq!(toks[2].kind, Kind::Switch);
}

#[test]
fn def_prefix_classifies_as_def_when_known_type() {
    let toks = lex("def_int def_string def_long");
    assert_eq!(toks[0].kind, Kind::Def);
    assert_eq!(toks[1].kind, Kind::Def);
    assert_eq!(toks[2].kind, Kind::Def);
}

#[test]
fn def_prefix_with_unknown_type_falls_back_to_identifier() {
    // `def_nope` isn't in `Type::from_def_str`, so it stays an Identifier.
    let toks = lex("def_nope");
    assert_eq!(toks[0].kind, Kind::Identifier);
}

#[test]
fn trigger_keywords_classify_as_trigger() {
    for trig in [
        "proc",
        "label",
        "queue",
        "timer",
        "softtimer",
        "opnpc1",
        "opheld5",
        "if_button",
        "if_close",
        "login",
        "logout",
    ] {
        let toks = lex(trig);
        assert_eq!(
            toks[0].kind,
            Kind::Trigger,
            "{} should classify as Trigger",
            trig
        );
    }
}

#[test]
fn unrecognised_words_are_plain_identifiers() {
    let toks = lex("foo bar baz_qux");
    assert_eq!(toks[0].kind, Kind::Identifier);
    assert_eq!(toks[1].kind, Kind::Identifier);
    assert_eq!(toks[2].kind, Kind::Identifier);
    assert_eq!(toks[2].value, "baz_qux");
}

// ── Variable-prefix tokens ──────────────────────────────────────────

#[test]
fn local_var_prefix() {
    let toks = lex("$local_x");
    assert_eq!(
        kinds(&toks),
        vec![Kind::LocalVar, Kind::Identifier, Kind::EndOfFile]
    );
}

#[test]
fn constant_var_prefix() {
    let toks = lex("^MAX_PLAYERS");
    assert_eq!(
        kinds(&toks),
        vec![Kind::ConstantVar, Kind::Identifier, Kind::EndOfFile]
    );
}

// ── Numeric literals ────────────────────────────────────────────────

#[test]
fn integer_literal() {
    let toks = lex("42");
    assert_eq!(toks[0].kind, Kind::Number);
    assert_eq!(toks[0].value, "42");
}

#[test]
fn long_literal_with_suffix() {
    // The lexer emits Kind::LongLiteral when read_number's bool flag is set.
    let toks = lex("123L");
    assert_eq!(toks[0].kind, Kind::LongLiteral);
}

#[test]
fn hex_literal_parses_as_number() {
    let toks = lex("0xFF");
    assert_eq!(toks[0].kind, Kind::Number);
    // 0xFF as i32 = 255.
    assert_eq!(toks[0].value, "255");
}

#[test]
fn hex_literal_with_high_bit_wraps_as_signed() {
    // 0xFFFFFFFF as u32 then reinterpreted as i32 → -1.
    let toks = lex("0xFFFFFFFF");
    assert_eq!(toks[0].kind, Kind::Number);
    assert_eq!(toks[0].value, "-1");
}

#[test]
fn number_immediately_followed_by_letters_is_identifier() {
    // e.g. "2dose1" used in obj names.
    let toks = lex("2dose1");
    assert_eq!(toks[0].kind, Kind::Identifier);
    assert_eq!(toks[0].value, "2dose1");
}

// ── Coord literals ──────────────────────────────────────────────────

#[test]
fn coord_literal_parses_as_coord() {
    let toks = lex("0_50_50_10_10");
    assert_eq!(toks[0].kind, Kind::CoordLiteral);
    assert_eq!(toks[0].value, "0_50_50_10_10");
}

#[test]
fn coord_shaped_with_trailing_identifier_is_identifier() {
    // `0_48_48_newbiefishing` is a name, not a coord — there's no 5th
    // numeric component.
    let toks = lex("0_48_48_newbiefishing");
    assert_eq!(toks[0].kind, Kind::Identifier);
    assert_eq!(toks[0].value, "0_48_48_newbiefishing");
}

// ── String literals ─────────────────────────────────────────────────

#[test]
fn simple_string_literal() {
    let toks = lex(r#""hello world""#);
    assert_eq!(toks[0].kind, Kind::StringLiteral);
    assert_eq!(toks[0].value, "hello world");
}

#[test]
fn empty_string_literal() {
    let toks = lex(r#""""#);
    assert_eq!(toks[0].kind, Kind::StringLiteral);
    assert_eq!(toks[0].value, "");
}

#[test]
fn string_literal_with_interpolation_block_keeps_inner_text() {
    // `<col=ff0000>` inside a string is the RuneScript interpolation
    // syntax. Inside the angle brackets, `"` is allowed.
    let toks = lex(r#""hello <"world">""#);
    assert_eq!(toks[0].kind, Kind::StringLiteral);
    // Whatever the lexer captures, it must NOT terminate at the inner `"`.
    assert!(
        toks[0].value.contains("world"),
        "interpolated content lost: {:?}",
        toks[0].value
    );
    assert_eq!(toks[1].kind, Kind::EndOfFile);
}

#[test]
fn unterminated_string_errors() {
    let msg = lex_err("\"never closes");
    assert!(
        msg.to_lowercase().contains("string") || msg.to_lowercase().contains("unterminated"),
        "expected unterminated-string error, got: {msg}"
    );
}

// ── Char literals ───────────────────────────────────────────────────

#[test]
fn char_literal_lexes() {
    let toks = lex("'x'");
    assert_eq!(toks[0].kind, Kind::CharLiteral);
}

// ── Comments ────────────────────────────────────────────────────────

#[test]
fn single_line_comment_is_emitted_as_token() {
    let toks = lex("// this is a comment\n42");
    assert_eq!(toks[0].kind, Kind::SingleLineComment);
    assert_eq!(toks[1].kind, Kind::Number);
}

#[test]
fn multi_line_comment_is_emitted_as_token() {
    let toks = lex("/* multi\nline */\n42");
    assert_eq!(toks[0].kind, Kind::MultiLineComment);
    assert_eq!(toks[1].kind, Kind::Number);
}

#[test]
fn unterminated_multi_line_comment_errors() {
    let msg = lex_err("/* never closes");
    assert!(
        msg.to_lowercase().contains("comment") || msg.to_lowercase().contains("unterminated"),
        "expected unterminated-comment error, got: {msg}"
    );
}

// ── Line / column tracking ──────────────────────────────────────────

#[test]
fn token_positions_on_first_line() {
    // Cols are 1-indexed.
    let toks = lex("[ ]");
    assert_eq!((toks[0].line, toks[0].column), (1, 1));
    assert_eq!((toks[1].line, toks[1].column), (1, 3));
}

#[test]
fn line_increments_after_newline() {
    let toks = lex("foo\nbar");
    assert_eq!(toks[0].line, 1);
    assert_eq!(toks[1].line, 2);
    assert_eq!(toks[1].column, 1);
}

#[test]
fn multiline_script_header_positions() {
    let src = "[proc,foo]\n  return;\n";
    let toks = lex(src);
    // First token `[` at 1:1, last meaningful token `;` somewhere on line 2.
    assert_eq!(toks[0].kind, Kind::LBracket);
    assert_eq!((toks[0].line, toks[0].column), (1, 1));
    let semicolon = toks
        .iter()
        .find(|t| t.kind == Kind::Semicolon)
        .expect("expected a semicolon");
    assert_eq!(semicolon.line, 2);
}

// ── End-to-end script header ────────────────────────────────────────

#[test]
fn full_script_header_token_sequence() {
    // Locks in the canonical token shape of a typed proc header. Every
    // downstream phase parses against this shape.
    let toks = lex("[proc,double](int $x)(int)");
    let expected = vec![
        Kind::LBracket,
        Kind::Trigger,    // proc
        Kind::Comma,
        Kind::Identifier, // double
        Kind::RBracket,
        Kind::LParen,
        Kind::Identifier, // int
        Kind::LocalVar,   // $
        Kind::Identifier, // x
        Kind::RParen,
        Kind::LParen,
        Kind::Identifier, // int
        Kind::RParen,
        Kind::EndOfFile,
    ];
    assert_eq!(kinds(&toks), expected);
}

// ── Errors ──────────────────────────────────────────────────────────

#[test]
fn unrecognised_character_errors() {
    let msg = lex_err("`");
    assert!(
        msg.contains("Unrecognized") || msg.to_lowercase().contains("unrecognized"),
        "expected unrecognised-char error, got: {msg}"
    );
}
