use crate::diagnostic_messages as msg;
use crate::error::SyntaxError;
use crate::lexer::Lexer;
use crate::token::{Kind, Token};
use crate::types::Type;
use std::path::PathBuf;

// ──────────────────────────── AST ────────────────────────────

#[derive(Debug, Clone)]
pub struct ScriptFile {
    pub scripts: Vec<ScriptDeclaration>,
}

#[derive(Debug, Clone)]
pub struct ScriptDeclaration {
    pub trigger: String,
    pub name: String,
    pub params: Vec<ParamDef>,
    pub return_types: Vec<Type>,
    pub body: Vec<Statement>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct ParamDef {
    pub param_type: Type,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum Statement {
    VarDeclaration {
        var_type: Type,
        name: String,
        value: Option<Expr>,
        line: usize,
    },
    ArrayDeclaration {
        element_type: Type,
        name: String,
        size: Expr,
        line: usize,
    },
    Assignment {
        target: Expr,
        value: Expr,
        line: usize,
    },
    If {
        condition: Expr,
        body: Vec<Statement>,
        else_if: Vec<(Expr, Vec<Statement>, usize)>,
        else_body: Option<Vec<Statement>>,
        /// Line of the `else` keyword (0 if no plain else block).
        else_line: usize,
        line: usize,
    },
    While {
        condition: Expr,
        body: Vec<Statement>,
        line: usize,
    },
    Switch {
        switch_type: String,
        expr: Expr,
        cases: Vec<SwitchCase>,
        default: Option<Vec<Statement>>,
        /// Index where `case default` appeared among all case entries in source order.
        /// 0 = default was first, cases.len() = default was last (or absent).
        /// Used to emit default body at the correct source position.
        default_index: usize,
        line: usize,
    },
    Return {
        values: Vec<Expr>,
        line: usize,
    },
    Expression {
        expr: Expr,
        line: usize,
    },
    /// A `case` that appeared outside of a switch statement.
    /// Parsed for error recovery; the type checker emits CASE_WITHOUT_SWITCH.
    OrphanCase {
        case: SwitchCase,
        line: usize,
    },
    Empty,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub values: Vec<Expr>,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub enum Expr {
    IntLiteral(i32),
    LongLiteral(i64),
    StringLiteral(String),
    BoolLiteral(bool),
    NullLiteral,
    CharLiteral(char),
    CoordLiteral(i32),
    Identifier(String),
    LocalVar(String, usize),    // (name, source_line)
    GameVar(String, usize),     // (name, source_line)
    ConstantVar(String, usize), // (name, source_line)
    ArrayAccess {
        name: String,
        index: Box<Expr>,
    },
    BinaryOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    LogicalOp {
        op: LogicOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        /// Source line where the RHS starts (for per-line LineNumber emission).
        rhs_line: usize,
    },
    Calc(Box<Expr>),
    CommandCall {
        name: String,
        args: Vec<Expr>,
        /// Source line of each argument (for per-line LineNumber emission).
        arg_lines: Vec<usize>,
        /// Source line of the call expression itself (for re-emission after multi-line args).
        call_line: usize,
    },
    ProcCall {
        name: String,
        args: Vec<Expr>,
        /// Source line of each argument (for per-line LineNumber emission).
        arg_lines: Vec<usize>,
        /// Source line of the call expression itself.
        call_line: usize,
    },
    JumpCall {
        name: String,
        args: Vec<Expr>,
        /// Source line of each argument (for per-line LineNumber emission).
        arg_lines: Vec<usize>,
        /// Source line of the call expression itself.
        call_line: usize,
    },
    JoinedString {
        parts: Vec<StringPart>,
    },
    /// Multiple assignment targets: $a, $b = multi_return_call(...)
    /// Contains (targets, target_lines) for per-expression LineNumber emission.
    MultiAssign(Vec<Expr>, Vec<usize>),
}

#[derive(Debug, Clone)]
pub enum StringPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    BitAnd,
    BitOr,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogicOp {
    And,
    Or,
}

// ──────────────────────────── Parser ────────────────────────────

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    file_path: PathBuf,
    is_sub_parser: bool,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, file_name: &PathBuf) -> Self {
        Self {
            tokens,
            pos: 0,
            file_path: file_name.clone(),
            is_sub_parser: false,
        }
    }

    // ── Token navigation ──

    fn at(&self) -> &Token {
        // Skip comments
        let mut idx = self.pos;
        while idx < self.tokens.len() {
            match self.tokens[idx].kind {
                Kind::SingleLineComment | Kind::MultiLineComment => idx += 1,
                _ => return &self.tokens[idx],
            }
        }
        self.tokens.last().unwrap() // EOF
    }

    fn peek_kind(&self) -> Kind {
        self.at().kind.clone()
    }

    fn next(&mut self) -> Token {
        loop {
            if self.pos >= self.tokens.len() {
                return self.tokens.last().unwrap().clone();
            }
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            match tok.kind {
                Kind::SingleLineComment | Kind::MultiLineComment => continue,
                _ => return tok,
            }
        }
    }

    fn expect(&mut self, kind: Kind) -> Result<Token, SyntaxError> {
        let tok = self.at().clone();
        if tok.kind != kind {
            return Err(SyntaxError::from_token(
                self.file_path.clone(),
                &tok,
                format!("Expected {:?}, got {:?} '{}'", kind, tok.kind, tok.value),
            ));
        }
        Ok(self.next())
    }

    fn is_eof(&self) -> bool {
        self.at().kind == Kind::EndOfFile
    }

    fn line(&self) -> usize {
        self.at().line
    }

    /// Peek at the nth non-comment token after the current position (0 = current).
    fn peek_kind_ahead(&self, n: usize) -> Kind {
        let mut idx = self.pos;
        let mut count = 0;
        loop {
            if idx >= self.tokens.len() {
                return Kind::EndOfFile;
            }
            match self.tokens[idx].kind {
                Kind::SingleLineComment | Kind::MultiLineComment => {
                    idx += 1;
                    continue;
                }
                _ => {
                    if count == n {
                        return self.tokens[idx].kind.clone();
                    }
                    count += 1;
                    idx += 1;
                }
            }
        }
    }

    // ── Top-level parsing ──

    pub fn parse(&mut self) -> Result<ScriptFile, SyntaxError> {
        let mut scripts = Vec::new();
        while !self.is_eof() {
            scripts.push(self.parse_script_declaration()?);
        }
        Ok(ScriptFile { scripts })
    }

    fn parse_script_declaration(&mut self) -> Result<ScriptDeclaration, SyntaxError> {
        let line = self.line();

        self.expect(Kind::LBracket)?;

        // Parse trigger type
        let trigger_tok = self.next();
        let trigger = trigger_tok.value.clone();

        self.expect(Kind::Comma)?;

        // Parse script name (may contain underscores, identifiers, dots)
        let name = self.parse_script_name()?;

        self.expect(Kind::RBracket)?;

        // Parse optional parameter list
        let params = if self.peek_kind() == Kind::LParen {
            self.parse_param_list()?
        } else {
            Vec::new()
        };

        // Parse optional return type list
        let return_types = if self.peek_kind() == Kind::LParen {
            self.parse_return_types()?
        } else {
            Vec::new()
        };

        // Parse body statements until next script declaration or EOF
        // Some scripts wrap the body in braces: [proc,name] { ... }
        let has_brace = self.peek_kind() == Kind::LBrace;
        if has_brace {
            self.next(); // consume '{'
        }
        let mut body = Vec::new();
        loop {
            if self.is_eof() {
                break;
            }
            if has_brace && self.peek_kind() == Kind::RBrace {
                self.next(); // consume '}'
                break;
            }
            if !has_brace && self.peek_kind() == Kind::LBracket {
                break;
            }
            body.push(self.parse_statement()?);
        }

        Ok(ScriptDeclaration {
            trigger,
            name,
            params,
            return_types,
            body,
            line,
        })
    }

    fn parse_script_name(&mut self) -> Result<String, SyntaxError> {
        let mut name = String::new();

        // Allow optional leading dot for dot-prefixed names like .huntnext
        if self.peek_kind() == Kind::Dot {
            self.next();
            name.push('.');
        }

        // First part can be identifier, number, underscore, or any keyword used as name
        let tok = self.next();
        match tok.kind {
            Kind::Identifier
            | Kind::Trigger
            | Kind::Calc
            | Kind::Command
            | Kind::BooleanTrue
            | Kind::BooleanFalse
            | Kind::Null
            | Kind::If
            | Kind::While
            | Kind::Else
            | Kind::Switch
            | Kind::Case
            | Kind::Default
            | Kind::Return
            | Kind::Def
            | Kind::Number
            | Kind::CoordLiteral => {
                name.push_str(&tok.value);
            }
            Kind::Underscore => {
                name.push('_');
            }
            _ => {
                return Err(SyntaxError::from_token(
                    self.file_path.clone(),
                    &tok,
                    format!("Expected script name, got {:?}", tok.kind),
                ));
            }
        }

        // Continue reading name parts (identifiers, underscores, numbers, colons for component refs)
        loop {
            match self.peek_kind() {
                Kind::Underscore => {
                    self.next();
                    name.push('_');
                    // If next is an identifier or number, consume it as part of name
                    match self.peek_kind() {
                        Kind::Identifier
                        | Kind::Trigger
                        | Kind::Command
                        | Kind::Calc
                        | Kind::BooleanTrue
                        | Kind::BooleanFalse
                        | Kind::Null
                        | Kind::If
                        | Kind::While
                        | Kind::Else
                        | Kind::Switch
                        | Kind::Case
                        | Kind::Default
                        | Kind::Return
                        | Kind::Def
                        | Kind::Number => {
                            let tok = self.next();
                            name.push_str(&tok.value);
                        }
                        _ => {}
                    }
                }
                Kind::Identifier => {
                    // Identifiers can follow numbers in names like: 0 + _45_152_lavafish
                    // (the lexer splits them into separate tokens)
                    let tok = self.next();
                    name.push_str(&tok.value);
                }
                Kind::Colon => {
                    // component:field style names like leather_crafting:com_88
                    self.next();
                    name.push(':');
                    let part = self.next();
                    name.push_str(&part.value);
                }
                Kind::BinaryOperator if self.at().value == "*" => {
                    self.next();
                    name.push('*');
                }
                Kind::BinaryOperator if self.at().value == "+" => {
                    self.next();
                    name.push('+');
                }
                _ => break,
            }
        }

        Ok(name)
    }

    fn parse_param_list(&mut self) -> Result<Vec<ParamDef>, SyntaxError> {
        self.expect(Kind::LParen)?;
        let mut params = Vec::new();

        while self.peek_kind() != Kind::RParen {
            if !params.is_empty() {
                self.expect(Kind::Comma)?;
            }

            // Parse type
            let type_tok = self.next();
            let param_type = Type::from_name(&type_tok.value).ok_or_else(|| {
                SyntaxError::from_token(
                    self.file_path.clone(),
                    &type_tok,
                    format!("Unknown parameter type: {}", type_tok.value),
                )
            })?;

            // Parse $name
            self.expect(Kind::LocalVar)?;
            let name_tok = self.next();
            let name = name_tok.value.clone();

            params.push(ParamDef { param_type, name });
        }

        self.expect(Kind::RParen)?;
        Ok(params)
    }

    fn parse_return_types(&mut self) -> Result<Vec<Type>, SyntaxError> {
        self.expect(Kind::LParen)?;
        let mut types = Vec::new();

        while self.peek_kind() != Kind::RParen {
            if !types.is_empty() {
                self.expect(Kind::Comma)?;
            }
            let tok = self.next();
            let ty = Type::from_name(&tok.value).ok_or_else(|| {
                SyntaxError::from_token(
                    self.file_path.clone(),
                    &tok,
                    format!("Unknown return type: {}", tok.value),
                )
            })?;
            types.push(ty);
        }

        self.expect(Kind::RParen)?;
        Ok(types)
    }

    // ── Statement parsing ──

    fn parse_statement(&mut self) -> Result<Statement, SyntaxError> {
        match self.peek_kind() {
            // Empty statement (trailing semicolon after switch/if block, etc.)
            Kind::Semicolon => {
                self.next(); // consume ';'
                Ok(Statement::Empty)
            }
            Kind::Def => self.parse_var_declaration(),
            Kind::If => self.parse_if_statement(),
            Kind::While => self.parse_while_statement(),
            Kind::Switch => self.parse_switch_statement(),
            Kind::Return => self.parse_return_statement(),
            Kind::Case => self.parse_orphan_case(),
            Kind::LocalVar => self.parse_assignment_or_expression(),
            Kind::GameVar => self.parse_game_var_assignment(),
            // Dot-prefixed statements: .%var = value (secondary entity game var assignment)
            // or .command(args) (secondary entity command call - expression statement).
            Kind::Dot => self.parse_assignment_or_expression(),
            _ => self.parse_expression_statement(),
        }
    }

    fn parse_var_declaration(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        let def_tok = self.next(); // consume def_xxx
        let var_type = Type::from_def_str(&def_tok.value).ok_or_else(|| {
            SyntaxError::from_token(
                self.file_path.clone(),
                &def_tok,
                format!("Unknown type definition: {}", def_tok.value),
            )
        })?;

        // Expect $varname
        self.expect(Kind::LocalVar)?;
        let name_tok = self.next();
        let name = name_tok.value.clone();

        // Check for array declaration: def_int $arr(size)
        if self.peek_kind() == Kind::LParen {
            self.expect(Kind::LParen)?;
            let size = self.parse_expression()?;
            self.expect(Kind::RParen)?;
            if self.peek_kind() == Kind::Semicolon {
                self.next();
            }
            return Ok(Statement::ArrayDeclaration {
                element_type: var_type,
                name,
                size,
                line,
            });
        }

        // Check for initialization
        let value = if self.peek_kind() == Kind::Equals {
            self.next(); // consume =
            let expr = self.parse_expression()?;
            Some(expr)
        } else {
            None
        };

        if self.peek_kind() == Kind::Semicolon {
            self.next();
        }

        Ok(Statement::VarDeclaration {
            var_type,
            name,
            value,
            line,
        })
    }

    fn parse_if_statement(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        self.expect(Kind::If)?;
        self.expect(Kind::LParen)?;
        let condition = self.parse_expression()?;
        self.expect(Kind::RParen)?;

        let body = self.parse_block_or_stmt()?;

        // Parse else-if and else chains
        let mut else_if = Vec::new();
        let mut else_body = None;
        let mut else_line = 0usize;

        while self.peek_kind() == Kind::Else {
            let el = self.line();
            self.next(); // consume 'else'
            if self.peek_kind() == Kind::If {
                // else if
                self.next(); // consume 'if'
                self.expect(Kind::LParen)?;
                let cond = self.parse_expression()?;
                self.expect(Kind::RParen)?;
                let body = self.parse_block_or_stmt()?;
                else_if.push((cond, body, el));
            } else {
                // else
                else_line = el;
                else_body = Some(self.parse_block_or_stmt()?);
                break;
            }
        }

        Ok(Statement::If {
            condition,
            body,
            else_if,
            else_body,
            else_line,
            line,
        })
    }

    fn parse_while_statement(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        self.expect(Kind::While)?;
        self.expect(Kind::LParen)?;
        let condition = self.parse_expression()?;
        self.expect(Kind::RParen)?;
        let body = self.parse_block_or_stmt()?;

        Ok(Statement::While {
            condition,
            body,
            line,
        })
    }

    fn parse_switch_statement(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        let switch_tok = self.next(); // consume switch_int/switch_str/etc.
        let switch_type = switch_tok.value.clone();

        self.expect(Kind::LParen)?;
        let expr = self.parse_expression()?;
        self.expect(Kind::RParen)?;
        self.expect(Kind::LBrace)?;

        let mut cases = Vec::new();
        let mut default = None;
        let mut default_index: usize = 0;

        while self.peek_kind() != Kind::RBrace {
            if self.peek_kind() == Kind::Case {
                self.next(); // consume 'case'

                // Handle `case default:` as the default case
                if self.peek_kind() == Kind::Default {
                    let default_tok = self.next(); // consume 'default'
                    if default.is_some() {
                        return Err(SyntaxError::from_token(
                            self.file_path.clone(),
                            &default_tok,
                            msg::SWITCH_DUPLICATE_DEFAULT.to_string(),
                        ));
                    }
                    self.expect(Kind::Colon)?;
                    default = Some(self.parse_case_body()?);
                    default_index = cases.len();
                    continue;
                }

                let mut values = Vec::new();
                loop {
                    let mut val = self.parse_expression()?;
                    // Handle component:name compound references in case values.
                    // Pattern: Identifier ':' Identifier (then ':' or ',' as separator)
                    // Only consume if after the second identifier there's ':' or ',' (not expression-start).
                    if let Expr::Identifier(ref name) = val {
                        if self.peek_kind() == Kind::Colon {
                            let after_colon = self.peek_kind_ahead(1);
                            let after_name = self.peek_kind_ahead(2);
                            // Form compound if followed by case separator ':' or next case value ','
                            if matches!(
                                after_colon,
                                Kind::Identifier | Kind::Number | Kind::Trigger | Kind::Command
                            ) && matches!(after_name, Kind::Colon | Kind::Comma)
                            {
                                let name = name.clone();
                                self.next(); // consume ':'
                                let part = self.next();
                                val = Expr::Identifier(format!("{}:{}", name, part.value));
                            }
                        }
                    }
                    values.push(val);
                    if self.peek_kind() == Kind::Comma {
                        self.next();
                    } else {
                        break;
                    }
                }

                self.expect(Kind::Colon)?;
                let body = self.parse_case_body()?;
                cases.push(SwitchCase { values, body });
            } else if self.peek_kind() == Kind::Default {
                let default_tok = self.next(); // consume 'default'
                if default.is_some() {
                    return Err(SyntaxError::from_token(
                        self.file_path.clone(),
                        &default_tok,
                        msg::SWITCH_DUPLICATE_DEFAULT.to_string(),
                    ));
                }
                self.expect(Kind::Colon)?;
                default = Some(self.parse_case_body()?);
            } else {
                let tok = self.at().clone();
                return Err(SyntaxError::from_token(
                    self.file_path.clone(),
                    &tok,
                    format!("Expected 'case' or 'default' in switch, got {:?}", tok.kind),
                ));
            }
        }

        self.expect(Kind::RBrace)?;

        Ok(Statement::Switch {
            switch_type,
            expr,
            cases,
            default,
            default_index,
            line,
        })
    }

    fn parse_case_body(&mut self) -> Result<Vec<Statement>, SyntaxError> {
        // Handle braced case bodies: case X : { stmt; stmt; }
        if self.peek_kind() == Kind::LBrace {
            return self.parse_block();
        }
        let mut stmts = Vec::new();
        while !self.is_eof()
            && self.peek_kind() != Kind::Case
            && self.peek_kind() != Kind::Default
            && self.peek_kind() != Kind::RBrace
        {
            stmts.push(self.parse_statement()?);
        }
        Ok(stmts)
    }

    fn parse_orphan_case(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        self.next(); // consume 'case'

        let mut values = Vec::new();
        if self.peek_kind() == Kind::Default {
            self.next(); // consume 'default'
        } else {
            values.push(self.parse_expression()?);
            while self.peek_kind() == Kind::Comma {
                self.next();
                values.push(self.parse_expression()?);
            }
        }

        if self.peek_kind() == Kind::Colon {
            self.next();
        }

        let body = self.parse_case_body()?;
        Ok(Statement::OrphanCase {
            case: SwitchCase { values, body },
            line,
        })
    }

    fn parse_return_statement(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        self.expect(Kind::Return)?;

        // return or return(expr, expr, ...)
        let mut values = Vec::new();
        if self.peek_kind() == Kind::LParen {
            self.expect(Kind::LParen)?;
            while self.peek_kind() != Kind::RParen {
                if !values.is_empty() {
                    self.expect(Kind::Comma)?;
                }
                values.push(self.parse_expression()?);
            }
            self.expect(Kind::RParen)?;
        }

        if self.peek_kind() == Kind::Semicolon {
            self.next();
        }

        Ok(Statement::Return { values, line })
    }

    fn parse_assignment_or_expression(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        let expr = self.parse_primary()?;

        // Multi-return assignment: $a, $b = expr
        if self.peek_kind() == Kind::Comma {
            let mut targets = vec![expr];
            let mut target_lines = vec![line];
            while self.peek_kind() == Kind::Comma {
                self.next(); // consume ','
                let tgt_line = self.line();
                let target = self.parse_primary()?;
                targets.push(target);
                target_lines.push(tgt_line);
            }
            if self.peek_kind() == Kind::Equals {
                self.next(); // consume '='
                let value = self.parse_expression()?;
                if self.peek_kind() == Kind::Semicolon {
                    self.next();
                }
                return Ok(Statement::Assignment {
                    target: Expr::MultiAssign(targets, target_lines),
                    value,
                    line,
                });
            }
            // Not an assignment; this is unusual but handle gracefully
            if self.peek_kind() == Kind::Semicolon {
                self.next();
            }
            return Ok(Statement::Expression {
                expr: targets.remove(0),
                line,
            });
        }

        if self.peek_kind() == Kind::Equals {
            self.next(); // consume =
            let value = self.parse_expression()?;
            if self.peek_kind() == Kind::Semicolon {
                self.next();
            }
            return Ok(Statement::Assignment {
                target: expr,
                value,
                line,
            });
        }

        if self.peek_kind() == Kind::Semicolon {
            self.next();
        }

        Ok(Statement::Expression { expr, line })
    }

    fn parse_game_var_assignment(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        let expr = self.parse_primary()?;

        // Multi-return assignment: %a, %b = expr  or  %a, $b = expr
        if self.peek_kind() == Kind::Comma {
            let mut targets = vec![expr];
            let mut target_lines = vec![line];
            while self.peek_kind() == Kind::Comma {
                self.next(); // consume ','
                let tgt_line = self.line();
                let target = self.parse_primary()?;
                targets.push(target);
                target_lines.push(tgt_line);
            }
            if self.peek_kind() == Kind::Equals {
                self.next();
                let value = self.parse_expression()?;
                if self.peek_kind() == Kind::Semicolon {
                    self.next();
                }
                return Ok(Statement::Assignment {
                    target: Expr::MultiAssign(targets, target_lines),
                    value,
                    line,
                });
            }
            // Not an assignment — treat as expression statement with first target
            let first = targets.remove(0);
            if self.peek_kind() == Kind::Semicolon {
                self.next();
            }
            return Ok(Statement::Expression { expr: first, line });
        }

        if self.peek_kind() == Kind::Equals {
            self.next(); // consume =
            let value = self.parse_expression()?;
            if self.peek_kind() == Kind::Semicolon {
                self.next();
            }
            return Ok(Statement::Assignment {
                target: expr,
                value,
                line,
            });
        }

        if self.peek_kind() == Kind::Semicolon {
            self.next();
        }

        Ok(Statement::Expression { expr, line })
    }

    fn parse_expression_statement(&mut self) -> Result<Statement, SyntaxError> {
        let line = self.line();
        let expr = self.parse_expression()?;

        // Handle multi-return assignment: .%var1, .%var2 = expr, etc.
        if self.peek_kind() == Kind::Comma {
            let mut targets = vec![expr];
            let mut target_lines = vec![line];
            while self.peek_kind() == Kind::Comma {
                self.next(); // consume ','
                let tgt_line = self.line();
                targets.push(self.parse_primary()?);
                target_lines.push(tgt_line);
            }
            if self.peek_kind() == Kind::Equals {
                self.next();
                let value = self.parse_expression()?;
                if self.peek_kind() == Kind::Semicolon {
                    self.next();
                }
                return Ok(Statement::Assignment {
                    target: Expr::MultiAssign(targets, target_lines),
                    value,
                    line,
                });
            }
            // Not an assignment; consume semicolon if present
            if self.peek_kind() == Kind::Semicolon {
                self.next();
            }
            return Ok(Statement::Expression {
                expr: targets.remove(0),
                line,
            });
        }

        if self.peek_kind() == Kind::Semicolon {
            self.next();
        }
        Ok(Statement::Expression { expr, line })
    }

    fn parse_block(&mut self) -> Result<Vec<Statement>, SyntaxError> {
        self.expect(Kind::LBrace)?;
        let mut stmts = Vec::new();
        while self.peek_kind() != Kind::RBrace && !self.is_eof() {
            stmts.push(self.parse_statement()?);
        }
        self.expect(Kind::RBrace)?;
        Ok(stmts)
    }

    /// Parse either a braced block `{ ... }` or a single statement (braceless body).
    fn parse_block_or_stmt(&mut self) -> Result<Vec<Statement>, SyntaxError> {
        if self.peek_kind() == Kind::LBrace {
            self.parse_block()
        } else {
            let stmt = self.parse_statement()?;
            Ok(vec![stmt])
        }
    }

    // ── Expression parsing (precedence climbing) ──

    fn parse_expression(&mut self) -> Result<Expr, SyntaxError> {
        self.parse_logical_or()
    }

    fn parse_logical_or(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_logical_and()?;
        while self.peek_kind() == Kind::LogicalOr {
            self.next();
            let rhs_line = self.line();
            let right = self.parse_logical_and()?;
            left = Expr::LogicalOp {
                op: LogicOp::Or,
                lhs: Box::new(left),
                rhs: Box::new(right),
                rhs_line,
            };
        }
        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_comparison()?;
        while self.peek_kind() == Kind::LogicalAnd {
            self.next();
            let rhs_line = self.line();
            let right = self.parse_comparison()?;
            left = Expr::LogicalOp {
                op: LogicOp::And,
                lhs: Box::new(left),
                rhs: Box::new(right),
                rhs_line,
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_additive()?;

        match self.peek_kind() {
            Kind::ComparisonOperator => {
                let tok = self.next();
                let op = match tok.value.as_str() {
                    "<" => BinOp::Lt,
                    ">" => BinOp::Gt,
                    "<=" => BinOp::LtEq,
                    ">=" => BinOp::GtEq,
                    _ => unreachable!(),
                };
                let right = self.parse_additive()?;
                left = Expr::BinaryOp {
                    op,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                };
            }
            Kind::Equals => {
                self.next();
                let right = self.parse_additive()?;
                left = Expr::BinaryOp {
                    op: BinOp::Eq,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                };
            }
            Kind::Not => {
                // ! or != operator (RuneScript uses both forms for not-equal)
                self.next(); // consume !
                // Optional '=' for != form
                if self.peek_kind() == Kind::Equals {
                    self.next();
                }
                let right = self.parse_additive()?;
                left = Expr::BinaryOp {
                    op: BinOp::NotEq,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                };
            }
            _ => {}
        }

        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_multiplicative()?;

        while self.peek_kind() == Kind::BinaryOperator {
            let tok = self.at().clone();
            match tok.value.as_str() {
                "+" | "-" => {
                    self.next();
                    let op = if tok.value == "+" {
                        BinOp::Add
                    } else {
                        BinOp::Sub
                    };
                    let right = self.parse_multiplicative()?;
                    left = Expr::BinaryOp {
                        op,
                        lhs: Box::new(left),
                        rhs: Box::new(right),
                    };
                }
                _ => break,
            }
        }

        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, SyntaxError> {
        let mut left = self.parse_primary()?;

        loop {
            let tok = self.at().clone();
            match (tok.kind.clone(), tok.value.as_str()) {
                (Kind::BinaryOperator, "*") => {
                    self.next();
                    let right = self.parse_primary()?;
                    left = Expr::BinaryOp {
                        op: BinOp::Mul,
                        lhs: Box::new(left),
                        rhs: Box::new(right),
                    };
                }
                (Kind::BinaryOperator, "/") => {
                    self.next();
                    let right = self.parse_primary()?;
                    left = Expr::BinaryOp {
                        op: BinOp::Div,
                        lhs: Box::new(left),
                        rhs: Box::new(right),
                    };
                }
                (Kind::BinaryOperator, "%") => {
                    // % is the modulo operator in arithmetic contexts (inside calc)
                    self.next();
                    let right = self.parse_primary()?;
                    left = Expr::BinaryOp {
                        op: BinOp::Mod,
                        lhs: Box::new(left),
                        rhs: Box::new(right),
                    };
                }
                _ => break,
            }
        }

        Ok(left)
    }

    fn parse_primary(&mut self) -> Result<Expr, SyntaxError> {
        match self.peek_kind() {
            Kind::Number => {
                let tok = self.next();
                let value: i32 = tok.value.parse().map_err(|_| {
                    SyntaxError::from_token(
                        self.file_path.clone(),
                        &tok,
                        format!("Invalid integer literal: {}", tok.value),
                    )
                })?;
                Ok(Expr::IntLiteral(value))
            }

            Kind::LongLiteral => {
                let tok = self.next();
                let value: i64 = tok.value.parse().map_err(|_| {
                    SyntaxError::from_token(
                        self.file_path.clone(),
                        &tok,
                        format!("Invalid long literal: {}", tok.value),
                    )
                })?;
                Ok(Expr::LongLiteral(value))
            }

            Kind::StringLiteral => {
                let tok = self.next();
                // Check if string contains interpolation markers (<expr>)
                if tok.value.contains('<') && tok.value.contains('>') {
                    Ok(self.parse_interpolated_string(&tok.value))
                } else {
                    Ok(Expr::StringLiteral(tok.value))
                }
            }

            Kind::CharLiteral => {
                let tok = self.next();
                let ch = tok.value.chars().next().unwrap_or('\0');
                Ok(Expr::CharLiteral(ch))
            }

            Kind::CoordLiteral => {
                let tok = self.next();
                // Parse coord literal: level_bx_bz_lx_lz
                // Packed as: (level << 28) | ((bx*64 + lx) << 14) | (bz*64 + lz)
                let parts: Vec<&str> = tok.value.split('_').collect();
                let coord_val = if parts.len() == 5 {
                    let level: i32 = parts[0].parse().unwrap_or(0);
                    let bx: i32 = parts[1].parse().unwrap_or(0);
                    let bz: i32 = parts[2].parse().unwrap_or(0);
                    let lx: i32 = parts[3].parse().unwrap_or(0);
                    let lz: i32 = parts[4].parse().unwrap_or(0);
                    let x = bx * 64 + lx;
                    let z = bz * 64 + lz;
                    (level << 28) | (x << 14) | z
                } else {
                    0
                };
                Ok(Expr::CoordLiteral(coord_val))
            }

            Kind::BooleanTrue => {
                self.next();
                Ok(Expr::BoolLiteral(true))
            }

            Kind::BooleanFalse => {
                self.next();
                Ok(Expr::BoolLiteral(false))
            }

            Kind::Null => {
                self.next();
                Ok(Expr::NullLiteral)
            }

            Kind::LocalVar => {
                self.next(); // consume $
                let name_tok = self.next();
                let name = name_tok.value.clone();

                // Check for array access: $arr($idx)
                if self.peek_kind() == Kind::LParen {
                    self.expect(Kind::LParen)?;
                    let index = self.parse_expression()?;
                    self.expect(Kind::RParen)?;
                    return Ok(Expr::ArrayAccess {
                        name,
                        index: Box::new(index),
                    });
                }

                Ok(Expr::LocalVar(name, self.line()))
            }

            Kind::GameVar => {
                self.next(); // consume %
                let name_tok = self.next();
                Ok(Expr::GameVar(name_tok.value.clone(), self.line()))
            }

            Kind::ConstantVar => {
                let const_line = self.line();
                self.next(); // consume ^
                let name_tok = self.next();
                Ok(Expr::ConstantVar(name_tok.value.clone(), const_line))
            }

            Kind::Calc => {
                self.next(); // consume 'calc'
                self.expect(Kind::LParen)?;
                let expr = self.parse_expression()?;
                self.expect(Kind::RParen)?;
                Ok(Expr::Calc(Box::new(expr)))
            }

            Kind::ScriptCall => {
                let call_line = if self.is_sub_parser { 0 } else { self.line() };
                self.next(); // consume ~
                // Allow optional leading dot for ~.name
                let dot = if self.peek_kind() == Kind::Dot {
                    self.next();
                    "."
                } else {
                    ""
                };
                let name_tok = self.next();
                let name = format!("{}{}", dot, name_tok.value);

                let (args, arg_lines) = if self.peek_kind() == Kind::LParen {
                    self.parse_call_args()?
                } else {
                    (Vec::new(), Vec::new())
                };

                Ok(Expr::ProcCall {
                    name,
                    args,
                    arg_lines,
                    call_line,
                })
            }

            Kind::JumpCall => {
                let call_line = if self.is_sub_parser { 0 } else { self.line() };
                self.next(); // consume @
                // Allow optional leading dot for @.name
                let dot = if self.peek_kind() == Kind::Dot {
                    self.next();
                    "."
                } else {
                    ""
                };
                let name_tok = self.next();
                let name = format!("{}{}", dot, name_tok.value);

                let (args, arg_lines) = if self.peek_kind() == Kind::LParen {
                    self.parse_call_args()?
                } else {
                    (Vec::new(), Vec::new())
                };

                Ok(Expr::JumpCall {
                    name,
                    args,
                    arg_lines,
                    call_line,
                })
            }

            Kind::Dot => {
                self.next(); // consume '.'
                match self.peek_kind() {
                    Kind::GameVar => {
                        // .%varname — dot-prefixed game var (secondary pointer)
                        self.next(); // consume %
                        let name_tok = self.next();
                        Ok(Expr::GameVar(format!(".{}", name_tok.value), self.line()))
                    }
                    Kind::LocalVar => {
                        // .$varname — dot-prefixed local var (unusual but handle it)
                        self.next(); // consume $
                        let name_tok = self.next();
                        Ok(Expr::LocalVar(format!(".{}", name_tok.value), self.line()))
                    }
                    _ => {
                        // Dot-prefixed command call: .huntnext(args)
                        let call_line = if self.is_sub_parser { 0 } else { self.line() };
                        let name_tok = self.next();
                        let mut name = format!(".{}", name_tok.value);
                        // Allow * suffix for vararg commands like .queue*
                        if self.peek_kind() == Kind::BinaryOperator && self.at().value == "*" {
                            self.next();
                            name.push('*');
                        }
                        let (mut args, mut arg_lines) = if self.peek_kind() == Kind::LParen {
                            self.parse_call_args()?
                        } else {
                            (Vec::new(), Vec::new())
                        };
                        // Handle second arg group for vararg commands
                        if self.peek_kind() == Kind::LParen {
                            let (extra_args, extra_lines) = self.parse_call_args()?;
                            args.extend(extra_args);
                            arg_lines.extend(extra_lines);
                        }
                        Ok(Expr::CommandCall {
                            name,
                            args,
                            arg_lines,
                            call_line,
                        })
                    }
                }
            }

            Kind::Identifier | Kind::Command | Kind::Trigger => {
                let tok = self.next();
                let mut name = tok.value.clone();

                // Allow * suffix for vararg commands like queue*, longqueue*
                if self.peek_kind() == Kind::BinaryOperator && self.at().value == "*" {
                    self.next();
                    name.push('*');
                }

                // Check if this is a command call: name(args) or name(args)(extra_args)
                if self.peek_kind() == Kind::LParen {
                    let call_line = if self.is_sub_parser { 0 } else { self.line() };
                    let (mut args, mut arg_lines) = self.parse_call_args()?;
                    // Handle second arg group for vararg commands: queue*(a,b)(extra_args)
                    if self.peek_kind() == Kind::LParen {
                        let (extra_args, extra_lines) = self.parse_call_args()?;
                        args.extend(extra_args);
                        arg_lines.extend(extra_lines);
                    }
                    return Ok(Expr::CommandCall {
                        name,
                        args,
                        arg_lines,
                        call_line,
                    });
                }

                // Handle component:field compound identifiers (e.g., duel_confirm:before_rule_line1)
                // Only consume when the token AFTER the compound is a value terminator
                // (RParen, Comma, Semicolon) — not ':' (case separator) or '(' (statement start).
                if self.peek_kind() == Kind::Colon {
                    let after_colon = self.peek_kind_ahead(1);
                    let after_compound = self.peek_kind_ahead(2);
                    if matches!(
                        after_colon,
                        Kind::Identifier | Kind::Number | Kind::Trigger | Kind::Command
                    ) && matches!(
                        after_compound,
                        Kind::RParen | Kind::Comma | Kind::Semicolon | Kind::RBrace | Kind::Colon
                    ) {
                        self.next(); // consume ':'
                        let part = self.next();
                        return Ok(Expr::Identifier(format!("{}:{}", name, part.value)));
                    }
                }

                Ok(Expr::Identifier(name))
            }

            Kind::LParen => {
                self.next(); // consume (
                let expr = self.parse_expression()?;
                self.expect(Kind::RParen)?;
                Ok(expr)
            }

            Kind::BinaryOperator if self.at().value == "-" => {
                // Unary negation - constant fold numeric literals
                self.next();
                let expr = self.parse_primary()?;
                match expr {
                    Expr::IntLiteral(n) => Ok(Expr::IntLiteral(-n)),
                    Expr::LongLiteral(n) => Ok(Expr::LongLiteral(-n)),
                    other => Ok(Expr::BinaryOp {
                        op: BinOp::Sub,
                        lhs: Box::new(Expr::IntLiteral(0)),
                        rhs: Box::new(other),
                    }),
                }
            }

            _ => {
                let tok = self.at().clone();
                Err(SyntaxError::from_token(
                    self.file_path.clone(),
                    &tok,
                    format!("Unexpected token: {:?} '{}'", tok.kind, tok.value),
                ))
            }
        }
    }

    fn parse_call_args(&mut self) -> Result<(Vec<Expr>, Vec<usize>), SyntaxError> {
        self.expect(Kind::LParen)?;
        let mut args = Vec::new();
        let mut arg_lines = Vec::new();
        while self.peek_kind() != Kind::RParen {
            if !args.is_empty() {
                self.expect(Kind::Comma)?;
            }
            let arg_line = if self.is_sub_parser { 0 } else { self.line() };
            let expr = self.parse_expression()?;
            // Handle component:field or table:column compound reference inside call args.
            // This is only valid in argument position, not in switch cases or conditions.
            let expr = if let Expr::Identifier(ref name) = expr {
                if self.peek_kind() == Kind::Colon {
                    let after_colon = self.peek_kind_ahead(1);
                    if matches!(
                        after_colon,
                        Kind::Identifier | Kind::Number | Kind::Trigger | Kind::Command
                    ) {
                        let name = name.clone();
                        self.next(); // consume ':'
                        let second = self.next();
                        let compound = format!("{}:{}", name, second.value);
                        if self.peek_kind() == Kind::LParen {
                            let call_line = if self.is_sub_parser { 0 } else { self.line() };
                            let (inner_args, inner_arg_lines) = self.parse_call_args()?;
                            Expr::CommandCall {
                                name: compound,
                                args: inner_args,
                                arg_lines: inner_arg_lines,
                                call_line,
                            }
                        } else {
                            Expr::Identifier(compound)
                        }
                    } else {
                        expr
                    }
                } else {
                    expr
                }
            } else {
                expr
            };
            args.push(expr);
            arg_lines.push(arg_line);
        }
        self.expect(Kind::RParen)?;
        Ok((args, arg_lines))
    }

    /// Parse a single expression from a string slice, returning an Expr.
    /// Used for embedded expressions in string templates like `<tostring($var)>`.
    fn parse_string_embedded_expr(&self, expr_str: &str) -> Expr {
        if expr_str.starts_with('$') {
            return Expr::LocalVar(expr_str[1..].to_string(), 0);
        }
        if expr_str.starts_with('%') {
            return Expr::GameVar(expr_str[1..].to_string(), 0);
        }
        if expr_str.starts_with('^') {
            return Expr::ConstantVar(expr_str[1..].to_string(), 0);
        }
        // Try to parse as a full expression using a sub-parser.
        let dummy_path = self.file_path.clone();
        let mut lexer = Lexer::new(expr_str, &dummy_path);
        if let Ok(tokens) = lexer.tokenize() {
            if !tokens.is_empty() {
                let mut sub = Parser::new(tokens, &dummy_path);
                sub.is_sub_parser = true;
                if let Ok(expr) = sub.parse_expression() {
                    return expr;
                }
            }
        }
        // Fallback: treat as plain identifier
        Expr::Identifier(expr_str.to_string())
    }

    fn parse_interpolated_string(&self, raw: &str) -> Expr {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut chars = raw.chars().peekable();
        let mut depth = 0;
        let mut in_expr = false;
        let mut expr_str = String::new();

        while let Some(ch) = chars.next() {
            if in_expr {
                if ch == '>' && depth == 0 {
                    in_expr = false;
                    // Treat as a code expression if it looks like RS2 code:
                    // starts with $/%/^/~ OR contains ( (function call).
                    // Otherwise it's a literal formatting tag like <p,neutral> or <col=ff0000>.
                    // A tag is "code" if it looks like RS2 code rather than an HTML-like
                    // formatting tag. Variable/constant/proc prefixes and function calls are
                    // obviously code. A bare identifier like <displayname> or <npc_name> is
                    // also code (it refers to a command call), while tags containing commas,
                    // equals signs, spaces, or colons are formatting tags like <p,neutral> or
                    // <col=ff0000>.
                    // A plain identifier or .identifier (dot-prefixed for secondary entity)
                    // like <displayname> or <.displayname> is a command call.
                    let stripped = expr_str.strip_prefix('.').unwrap_or(expr_str.as_str());
                    let is_plain_identifier = !stripped.is_empty()
                        && stripped
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_');
                    let is_code = expr_str.starts_with('$')
                        || expr_str.starts_with('%')
                        || expr_str.starts_with('^')
                        || expr_str.starts_with('~')
                        || expr_str.contains('(')
                        || is_plain_identifier;
                    if is_code {
                        let expr = self.parse_string_embedded_expr(&expr_str);
                        parts.push(StringPart::Expr(expr));
                    } else {
                        // Literal tag: push "<tag>" as a separate literal string part
                        let tag_lit = format!("<{}>", expr_str);
                        parts.push(StringPart::Literal(tag_lit));
                    }
                    expr_str.clear();
                } else {
                    if ch == '(' {
                        depth += 1;
                    }
                    if ch == ')' {
                        depth -= 1;
                    }
                    expr_str.push(ch);
                }
            } else if ch == '<' {
                if !current.is_empty() {
                    parts.push(StringPart::Literal(current.clone()));
                    current.clear();
                }
                in_expr = true;
                depth = 0;
            } else {
                current.push(ch);
            }
        }

        if !current.is_empty() {
            parts.push(StringPart::Literal(current));
        }

        if parts.len() == 1 {
            match &parts[0] {
                StringPart::Literal(s) => return Expr::StringLiteral(s.clone()),
                // Single expression part (e.g., "<oc_name($obj)>") needs no JoinString.
                StringPart::Expr(e) => return e.clone(),
            }
        }

        Expr::JoinedString { parts }
    }
}

// ── Backward-compatible types for transition ──

pub type Script = ScriptFile;

/// Legacy AST kind for backward compatibility with existing tests
#[derive(Debug, Clone)]
pub enum AstKind {
    NumericLiteral(i32),
    StringLiteral(String),
    Identifier(String),
    Proc,
    BinaryExpression {
        lhs: Box<AstKind>,
        rhs: Box<AstKind>,
        operator: String,
    },
    Define {
        name: String,
        var_type: Type,
        value: Box<AstKind>,
    },
    Program,
    Trigger {
        name: Box<AstKind>,
        kind: Box<AstKind>,
        args: Vec<Box<AstKind>>,
        body: Box<AstKind>,
        return_type: Box<AstKind>,
    },
    Integer,
    LocalVar(String),
    ReturnType,
    Return(Box<AstKind>),
    ConditionalExpression {
        lhs: Box<AstKind>,
        rhs: Box<AstKind>,
        value: Box<AstKind>,
    },
    If {
        expression: Box<AstKind>,
        value: Box<AstKind>,
        return_statement: Box<AstKind>,
    },
    AssignmentExpression,
    While {
        condition: Box<AstKind>,
        body: Box<AstKind>,
    },
    Block(Vec<AstKind>),
    FunctionCall {
        name: String,
        arguments: Vec<Box<AstKind>>,
    },
    Assignment {
        target: Box<AstKind>,
        value: Box<AstKind>,
    },
    ScriptCall {
        script: Box<AstKind>,
        arguments: Vec<Box<AstKind>>,
    },
}

#[derive(Debug, Clone)]
pub enum ConfigType {
    Floor,
    IdKit,
    Location,
    Npc,
    Object,
    Sequence,
    Spotanim,
    Varp,
    Param,
    Enum,
    Struct,
}
