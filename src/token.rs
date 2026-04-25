#[derive(Debug, PartialEq, Clone)]
pub struct Token {
    pub line: usize,
    pub column: usize,
    pub kind: Kind,
    pub value: String,
}

impl Token {
    pub fn new(kind: Kind, value: String, line: usize, column: usize) -> Self {
        Token {
            line,
            column,
            kind,
            value,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Kind {
    // Brackets and delimiters
    LBracket,  // [
    RBracket,  // ]
    LParen,    // (
    RParen,    // )
    LBrace,    // {
    RBrace,    // }
    Semicolon, // ;
    Comma,     // ,
    Colon,     // :

    // Operators
    Equals,             // = (assignment)
    BinaryOperator,     // +, -, *, /, %
    ComparisonOperator, // <, >, <=, >=, =  (in comparison context)
    LogicalAnd,         // &
    LogicalOr,          // |
    Not,                // !

    // Special characters
    Underscore, // _
    Dot,        // . (for dot-prefixed command names)
    ScriptCall, // ~ (gosub operator)
    JumpCall,   // @ (jump operator)
    Hash,       // # (unused but lexed)

    // Keywords
    Trigger, // proc, clientscript, label, etc.
    Command, // engine commands
    Def,     // def_int, def_string, etc.
    Return,  // return
    If,      // if
    Else,    // else
    While,   // while
    Switch,  // switch_int, switch_str, etc.
    Case,    // case
    Default, // default
    Calc,    // calc
    Null,    // null

    // Identifiers and literals
    Identifier,    // Regular identifiers
    LocalVar,      // $ prefixed variables
    GameVar,       // % prefixed variables (varp/varn/vars)
    ConstantVar,   // ^ prefixed constants
    Number,        // Numeric literals (integer)
    LongLiteral,   // Long literals (suffixed with L)
    StringLiteral, // "string contents"
    BooleanTrue,   // true
    BooleanFalse,  // false
    CharLiteral,   // 'x'
    CoordLiteral,  // coordinate literal (0_0_0_0_0)

    // Comments
    SingleLineComment,
    MultiLineComment,

    // End of file token
    EndOfFile,
}
