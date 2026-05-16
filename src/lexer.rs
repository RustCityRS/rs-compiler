use crate::error::LexingError;
use crate::token::{Kind, Token};
use crate::types::Type;
use std::path::PathBuf;

pub struct Lexer<'a> {
    source: &'a [u8],
    file_name: &'a PathBuf,
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str, file_name: &'a PathBuf) -> Self {
        Self {
            source: input.as_bytes(),
            file_name,
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn peek_ahead(&self, offset: usize) -> Option<u8> {
        self.source.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.source.get(self.pos).copied()?;
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    fn token(&self, kind: Kind, value: String, line: usize, col: usize) -> Token {
        Token::new(kind, value, line, col)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_ascii_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn read_string_literal(&mut self) -> Result<Token, LexingError> {
        let start_line = self.line;
        let start_col = self.col;
        self.advance(); // consume opening quote
        let mut value = String::new();
        let mut interp_depth: u32 = 0; // depth inside <...> interpolation blocks
        loop {
            match self.advance() {
                Some(b'"') => {
                    // Inside an interpolation block, embedded quotes are allowed
                    if interp_depth > 0 {
                        value.push('"');
                    } else {
                        break;
                    }
                }
                Some(b'<') => {
                    interp_depth += 1;
                    value.push('<');
                }
                Some(b'>') => {
                    if interp_depth > 0 {
                        interp_depth -= 1;
                    }
                    value.push('>');
                }
                Some(b'\\') => match self.advance() {
                    // RuneScript only escapes \\, \", and \< (matching ANTLR grammar)
                    Some(b'\\') => value.push('\\'),
                    Some(b'"') => value.push('"'),
                    Some(b'<') => value.push('<'),
                    Some(ch) => {
                        value.push('\\');
                        value.push(ch as char);
                    }
                    None => {
                        return Err(LexingError::new(
                            self.file_name.clone(),
                            "Unterminated string literal".to_string(),
                            start_line,
                            start_col,
                        ));
                    }
                },
                Some(ch) => {
                    // Handle multi-byte UTF-8 sequences
                    if ch < 0x80 {
                        value.push(ch as char);
                    } else if ch >= 0xC0 {
                        // Start of multi-byte UTF-8: decode properly
                        let mut bytes = vec![ch];
                        while let Some(&next) = self.source.get(self.pos) {
                            if next >= 0x80 && next < 0xC0 {
                                bytes.push(next);
                                self.pos += 1;
                                self.col += 1;
                            } else {
                                break;
                            }
                        }
                        if let Ok(s) = std::str::from_utf8(&bytes) {
                            value.push_str(s);
                        } else {
                            // Fallback: push as Latin-1
                            for b in bytes {
                                value.push(b as char);
                            }
                        }
                    } else {
                        // Continuation byte without start — push as-is
                        value.push(ch as char);
                    }
                }
                None => {
                    return Err(LexingError::new(
                        self.file_name.clone(),
                        "Unterminated string literal".to_string(),
                        start_line,
                        start_col,
                    ));
                }
            }
        }
        Ok(self.token(Kind::StringLiteral, value, start_line, start_col))
    }

    fn read_char_literal(&mut self) -> Result<Token, LexingError> {
        let start_line = self.line;
        let start_col = self.col;
        self.advance(); // consume opening quote
        let ch = match self.advance() {
            Some(b'\\') => match self.advance() {
                Some(b'n') => '\n',
                Some(b't') => '\t',
                Some(b'\\') => '\\',
                Some(b'\'') => '\'',
                Some(c) => c as char,
                None => {
                    return Err(LexingError::new(
                        self.file_name.clone(),
                        "Unterminated character literal".to_string(),
                        start_line,
                        start_col,
                    ));
                }
            },
            Some(c) => c as char,
            None => {
                return Err(LexingError::new(
                    self.file_name.clone(),
                    "Unterminated character literal".to_string(),
                    start_line,
                    start_col,
                ));
            }
        };
        match self.advance() {
            Some(b'\'') => {}
            _ => {
                return Err(LexingError::new(
                    self.file_name.clone(),
                    "Expected closing single quote".to_string(),
                    self.line,
                    self.col,
                ));
            }
        }
        Ok(self.token(Kind::CharLiteral, ch.to_string(), start_line, start_col))
    }

    fn read_identifier(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.advance();
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.source[start..self.pos]).to_string()
    }

    fn read_number(&mut self) -> (String, bool) {
        let start = self.pos;
        let mut is_long = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        // Check for long suffix
        if self.peek() == Some(b'L') || self.peek() == Some(b'l') {
            is_long = true;
            self.advance();
        }
        let number = String::from_utf8_lossy(&self.source[start..self.pos]).to_string();
        // Strip the L suffix from the value
        let value = if is_long {
            number
                .trim_end_matches(|c| c == 'L' || c == 'l')
                .to_string()
        } else {
            number
        };
        (value, is_long)
    }

    fn read_single_line_comment(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == b'\n' {
                break;
            }
            self.advance();
        }
        String::from_utf8_lossy(&self.source[start..self.pos]).to_string()
    }

    fn read_multi_line_comment(&mut self) -> Result<String, LexingError> {
        let start_line = self.line;
        let start_col = self.col;
        let mut comment = String::new();
        let mut depth: u32 = 1;
        while depth > 0 {
            match self.advance() {
                Some(b'/') => {
                    if self.peek() == Some(b'*') {
                        self.advance();
                        comment.push_str("/*");
                        depth += 1;
                    } else {
                        comment.push('/');
                    }
                }
                Some(b'*') => {
                    if self.peek() == Some(b'/') {
                        self.advance();
                        depth -= 1;
                        if depth > 0 {
                            comment.push_str("*/");
                        }
                    } else {
                        comment.push('*');
                    }
                }
                Some(ch) => comment.push(ch as char),
                None => {
                    return Err(LexingError::new(
                        self.file_name.clone(),
                        "Unterminated multi-line comment".to_string(),
                        start_line,
                        start_col,
                    ));
                }
            }
        }
        Ok(comment)
    }

    fn classify_identifier(&self, ident: &str) -> Kind {
        match ident {
            "proc" | "clientscript" | "label" | "debugproc" | "walktrigger" | "queue" | "timer"
            | "softtimer" | "ai_queue" | "ai_timer" | "opnpc" | "opnpc1" | "opnpc2" | "opnpc3"
            | "opnpc4" | "opnpc5" | "opobj" | "opobj1" | "opobj2" | "opobj3" | "opobj4"
            | "opobj5" | "oploc" | "oploc1" | "oploc2" | "oploc3" | "oploc4" | "oploc5"
            | "opplayer" | "opplayer1" | "opplayer2" | "opplayer3" | "opplayer4" | "opplayer5"
            | "opheld" | "opheld1" | "opheld2" | "opheld3" | "opheld4" | "opheld5" | "oplocu"
            | "opobju" | "opnpcu" | "opplayeru" | "opheldu" | "opnpct" | "oploct" | "opplayert"
            | "opheldt" | "opobjt" | "apnpc" | "apnpc1" | "apnpc2" | "apnpc3" | "apnpc4"
            | "apnpc5" | "aploc" | "aploc1" | "aploc2" | "aploc3" | "aploc4" | "aploc5"
            | "apobj" | "apobj1" | "apobj2" | "apobj3" | "apobj4" | "apobj5" | "applayer"
            | "applayer1" | "applayer2" | "applayer3" | "applayer4" | "applayer5" | "if_button"
            | "if_close" | "login" | "logout" | "inv_button1" | "inv_button2" | "inv_button3"
            | "inv_button4" | "inv_button5" | "inv_button6" | "inv_button7" | "inv_button8"
            | "inv_button9" | "inv_button10" | "inv_buttond" | "mapzone" | "maplength"
            | "mapenter" => Kind::Trigger,

            "if" => Kind::If,
            "else" => Kind::Else,
            "while" => Kind::While,
            "return" => Kind::Return,
            "calc" => Kind::Calc,
            "null" => Kind::Null,
            "true" => Kind::BooleanTrue,
            "false" => Kind::BooleanFalse,
            "case" => Kind::Case,
            "default" => Kind::Default,

            s if s.starts_with("switch_") => Kind::Switch,

            s if s.starts_with("def_") => {
                if Type::from_def_str(s).is_some() {
                    Kind::Def
                } else {
                    Kind::Identifier
                }
            }

            _ => Kind::Identifier,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexingError> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace();

            let Some(ch) = self.peek() else {
                break;
            };

            let start_line = self.line;
            let start_col = self.col;

            match ch {
                b'[' => {
                    self.advance();
                    tokens.push(self.token(Kind::LBracket, "[".into(), start_line, start_col));
                }
                b']' => {
                    self.advance();
                    tokens.push(self.token(Kind::RBracket, "]".into(), start_line, start_col));
                }
                b'(' => {
                    self.advance();
                    tokens.push(self.token(Kind::LParen, "(".into(), start_line, start_col));
                }
                b')' => {
                    self.advance();
                    tokens.push(self.token(Kind::RParen, ")".into(), start_line, start_col));
                }
                b'{' => {
                    self.advance();
                    tokens.push(self.token(Kind::LBrace, "{".into(), start_line, start_col));
                }
                b'}' => {
                    self.advance();
                    tokens.push(self.token(Kind::RBrace, "}".into(), start_line, start_col));
                }
                b';' => {
                    self.advance();
                    tokens.push(self.token(Kind::Semicolon, ";".into(), start_line, start_col));
                }
                b',' => {
                    self.advance();
                    tokens.push(self.token(Kind::Comma, ",".into(), start_line, start_col));
                }
                b':' => {
                    self.advance();
                    tokens.push(self.token(Kind::Colon, ":".into(), start_line, start_col));
                }
                b'&' => {
                    self.advance();
                    tokens.push(self.token(Kind::LogicalAnd, "&".into(), start_line, start_col));
                }
                b'|' => {
                    self.advance();
                    tokens.push(self.token(Kind::LogicalOr, "|".into(), start_line, start_col));
                }
                b'!' => {
                    self.advance();
                    tokens.push(self.token(Kind::Not, "!".into(), start_line, start_col));
                }
                b'#' => {
                    self.advance();
                    tokens.push(self.token(Kind::Hash, "#".into(), start_line, start_col));
                }

                b'.' => {
                    self.advance();
                    tokens.push(self.token(Kind::Dot, ".".into(), start_line, start_col));
                }
                b'~' => {
                    self.advance();
                    tokens.push(self.token(Kind::ScriptCall, "~".into(), start_line, start_col));
                }
                b'@' => {
                    self.advance();
                    tokens.push(self.token(Kind::JumpCall, "@".into(), start_line, start_col));
                }

                b'$' => {
                    self.advance(); // consume $
                    tokens.push(self.token(Kind::LocalVar, "$".into(), start_line, start_col));
                }

                b'%' => {
                    self.advance(); // consume %
                    // Disambiguate GameVar prefix vs modulo operator by the
                    // following byte. `%name` is a varp/varn reference;
                    // `% 4` / `%4` after a value is modulo. If the next byte
                    // is whitespace or a digit (or EOF / operator), the token
                    // is binary `%`; only a letter/underscore kicks off a
                    // GameVar identifier.
                    let is_gamevar = match self.peek() {
                        Some(c) => c.is_ascii_alphabetic() || c == b'_',
                        None => false,
                    };
                    if is_gamevar {
                        tokens.push(self.token(Kind::GameVar, "%".into(), start_line, start_col));
                    } else {
                        tokens.push(self.token(
                            Kind::BinaryOperator,
                            "%".into(),
                            start_line,
                            start_col,
                        ));
                    }
                }

                b'^' => {
                    self.advance(); // consume ^
                    tokens.push(self.token(Kind::ConstantVar, "^".into(), start_line, start_col));
                }

                b'=' => {
                    self.advance();
                    tokens.push(self.token(Kind::Equals, "=".into(), start_line, start_col));
                }

                b'<' => {
                    self.advance();
                    if self.peek() == Some(b'=') {
                        self.advance();
                        tokens.push(self.token(
                            Kind::ComparisonOperator,
                            "<=".into(),
                            start_line,
                            start_col,
                        ));
                    } else {
                        tokens.push(self.token(
                            Kind::ComparisonOperator,
                            "<".into(),
                            start_line,
                            start_col,
                        ));
                    }
                }
                b'>' => {
                    self.advance();
                    if self.peek() == Some(b'=') {
                        self.advance();
                        tokens.push(self.token(
                            Kind::ComparisonOperator,
                            ">=".into(),
                            start_line,
                            start_col,
                        ));
                    } else {
                        tokens.push(self.token(
                            Kind::ComparisonOperator,
                            ">".into(),
                            start_line,
                            start_col,
                        ));
                    }
                }

                b'+' | b'-' | b'*' => {
                    self.advance();
                    // Handle negative numbers: if '-' follows certain tokens, it's unary
                    tokens.push(self.token(
                        Kind::BinaryOperator,
                        (ch as char).to_string(),
                        start_line,
                        start_col,
                    ));
                }

                b'/' => {
                    self.advance();
                    match self.peek() {
                        Some(b'/') => {
                            self.advance(); // consume second /
                            let comment = self.read_single_line_comment();
                            tokens.push(self.token(
                                Kind::SingleLineComment,
                                comment,
                                start_line,
                                start_col,
                            ));
                        }
                        Some(b'*') => {
                            self.advance(); // consume *
                            let comment = self.read_multi_line_comment()?;
                            tokens.push(self.token(
                                Kind::MultiLineComment,
                                comment,
                                start_line,
                                start_col,
                            ));
                        }
                        _ => {
                            tokens.push(self.token(
                                Kind::BinaryOperator,
                                "/".into(),
                                start_line,
                                start_col,
                            ));
                        }
                    }
                }

                b'"' => {
                    let tok = self.read_string_literal()?;
                    tokens.push(tok);
                }

                b'\'' => {
                    let tok = self.read_char_literal()?;
                    tokens.push(tok);
                }

                b'_' => {
                    // Could be standalone underscore or start of identifier
                    if self
                        .peek_ahead(1)
                        .map_or(false, |c| c.is_ascii_alphanumeric() || c == b'_')
                    {
                        let ident = self.read_identifier();
                        let kind = self.classify_identifier(&ident);
                        tokens.push(self.token(kind, ident, start_line, start_col));
                    } else {
                        self.advance();
                        tokens.push(self.token(
                            Kind::Underscore,
                            "_".into(),
                            start_line,
                            start_col,
                        ));
                    }
                }

                c if c.is_ascii_alphabetic() => {
                    let ident = self.read_identifier();
                    let kind = self.classify_identifier(&ident);
                    tokens.push(self.token(kind, ident, start_line, start_col));
                }

                c if c.is_ascii_digit() => {
                    // Check for hex literal 0x...
                    let (value, is_long) = if c == b'0'
                        && self.source.get(self.pos + 1).copied() == Some(b'x')
                    {
                        self.advance(); // consume '0'
                        self.advance(); // consume 'x'
                        let hex_start = self.pos;
                        while let Some(ch) = self.peek() {
                            if ch.is_ascii_hexdigit() {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let hex_str =
                            String::from_utf8_lossy(&self.source[hex_start..self.pos]).to_string();
                        // Parse as u32 first to handle 0xFFFFFFFF correctly, then reinterpret as i32
                        let val = u32::from_str_radix(&hex_str, 16).unwrap_or(0) as i32;
                        (val.to_string(), false)
                    } else {
                        self.read_number()
                    };
                    // Check if this number is immediately followed by more identifier chars
                    // (e.g., "2dose1strength" or "0_48_48_newbiefishing") → lex as Identifier
                    let next_is_ident = self.peek().map_or(false, |c| c.is_ascii_alphabetic());
                    if !is_long && next_is_ident {
                        // Number immediately followed by letters — treat as identifier
                        let mut ident = value;
                        let rest = self.read_identifier();
                        ident.push_str(&rest);
                        tokens.push(self.token(Kind::Identifier, ident, start_line, start_col));
                    } else if !is_long && self.peek() == Some(b'_') {
                        // Possibly a coord literal like 0_39_48_41_13
                        let saved_pos = self.pos;
                        let saved_line = self.line;
                        let saved_col = self.col;
                        let mut coord = value.clone();
                        let mut is_coord = true;
                        for _ in 0..4 {
                            if self.peek() == Some(b'_') {
                                self.advance(); // consume _
                                coord.push('_');
                                if self.peek().map_or(false, |c| c.is_ascii_digit()) {
                                    let (part, part_long) = self.read_number();
                                    if part_long {
                                        is_coord = false;
                                        break;
                                    }
                                    coord.push_str(&part);
                                } else {
                                    // Non-numeric after underscore — could be identifier suffix
                                    // e.g., 0_48_48_newbiefishing: read the rest as identifier
                                    is_coord = false;
                                    break;
                                }
                            } else {
                                is_coord = false;
                                break;
                            }
                        }
                        if is_coord
                            && !self
                                .peek()
                                .map_or(false, |c| c == b'_' || c.is_ascii_alphanumeric())
                        {
                            tokens.push(self.token(
                                Kind::CoordLiteral,
                                coord,
                                start_line,
                                start_col,
                            ));
                        } else {
                            // Not a coord literal; restore and re-read as identifier
                            self.pos = saved_pos;
                            self.line = saved_line;
                            self.col = saved_col;
                            // If next is '_' followed by alpha, read the whole thing as identifier
                            if self.peek() == Some(b'_') {
                                let mut ident = value;
                                // read_identifier starts from current pos which is '_'
                                let rest = self.read_identifier();
                                ident.push_str(&rest);
                                tokens.push(self.token(
                                    Kind::Identifier,
                                    ident,
                                    start_line,
                                    start_col,
                                ));
                            } else {
                                tokens.push(self.token(Kind::Number, value, start_line, start_col));
                            }
                        }
                    } else {
                        let kind = if is_long {
                            Kind::LongLiteral
                        } else {
                            Kind::Number
                        };
                        tokens.push(self.token(kind, value, start_line, start_col));
                    }
                }

                _ => {
                    return Err(LexingError::new(
                        self.file_name.clone(),
                        format!("Unrecognized character '{}'", ch as char),
                        self.line,
                        self.col,
                    ));
                }
            }
        }

        tokens.push(Token::new(
            Kind::EndOfFile,
            "EOF".into(),
            self.line,
            self.col,
        ));
        Ok(tokens)
    }
}
