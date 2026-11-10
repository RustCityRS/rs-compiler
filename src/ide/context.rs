use super::Symbol;
use super::index::Lexeme;
use crate::token::Kind;
use crate::types::Type;
use std::ops::Range;

/// Context for references, including incomplete documents that cannot be type checked.
pub(super) fn expected_types(
    tokens: &[&(Lexeme<'_>, Range<usize>)],
    bindings: &[Option<&Symbol>],
) -> Vec<Option<Type>> {
    let mut pairs = vec![None; tokens.len()];
    let mut stack = Vec::new();
    for (i, (token, _)) in tokens.iter().enumerate() {
        match token.kind {
            Kind::LParen | Kind::LBrace | Kind::LBracket => stack.push(i),
            Kind::RParen | Kind::RBrace | Kind::RBracket => {
                if let Some(start) = stack.pop() {
                    pairs[start] = Some(i);
                }
            }
            _ => {}
        }
    }
    let mut scan = Context {
        tokens,
        bindings,
        pairs,
        expected: vec![None; tokens.len()],
    };
    scan.block(0..tokens.len(), None, &[], 0);
    scan.expected
}

struct Context<'a> {
    tokens: &'a [&'a (Lexeme<'a>, Range<usize>)],
    bindings: &'a [Option<&'a Symbol>],
    pairs: Vec<Option<usize>>,
    expected: Vec<Option<Type>>,
}

impl Context<'_> {
    fn kind(&self, i: usize) -> Kind {
        self.tokens[i].0.kind
    }

    fn next(&self, i: usize) -> usize {
        self.pairs[i].unwrap_or(i) + 1
    }

    fn find(&self, range: Range<usize>, kinds: &[Kind]) -> Option<usize> {
        let mut i = range.start;
        while i < range.end {
            if kinds.contains(&self.kind(i)) {
                return Some(i);
            }
            i = self.next(i);
        }
        None
    }

    fn block(&mut self, range: Range<usize>, switch: Option<Type>, returns: &[Type], depth: usize) {
        if depth > 64 {
            return;
        }
        let mut returns = returns;
        let mut i = range.start;
        while i < range.end {
            match self.kind(i) {
                Kind::LBracket => {
                    let end = self.pairs[i].unwrap_or(i);
                    if let Some(symbol) = (i..=end).find_map(|j| self.bindings[j]) {
                        returns = &symbol.returns;
                    }
                    i = end + 1;
                    while i < range.end && self.kind(i) == Kind::LParen {
                        i = self.next(i);
                    }
                }
                Kind::LBrace => {
                    let end = self.pairs[i].unwrap_or(range.end);
                    self.block(i + 1..end, None, returns, depth + 1);
                    i = end + 1;
                }
                Kind::Switch | Kind::If | Kind::While => {
                    let switch_type = self.tokens[i]
                        .0
                        .value
                        .strip_prefix("switch_")
                        .and_then(Type::from_name);
                    i += 1;
                    if i < range.end && self.kind(i) == Kind::LParen {
                        let end = self.pairs[i].unwrap_or(range.end);
                        self.expression(i + 1..end, switch_type, depth + 1);
                        i = end + 1;
                    }
                    if i < range.end && self.kind(i) == Kind::LBrace {
                        let end = self.pairs[i].unwrap_or(range.end);
                        self.block(i + 1..end, switch_type, returns, depth + 1);
                        i = end + 1;
                    }
                }
                Kind::Case => {
                    let end = self
                        .find(i + 1..range.end, &[Kind::Colon])
                        .unwrap_or(range.end);
                    self.arguments(i + 1..end, &[], switch, depth + 1);
                    i = end + 1;
                }
                Kind::Return => {
                    i += 1;
                    if i < range.end && self.kind(i) == Kind::LParen {
                        let end = self.pairs[i].unwrap_or(range.end);
                        self.arguments(i + 1..end, returns, None, depth + 1);
                        i = end + 1;
                    }
                }
                Kind::Def => {
                    let ty = self.tokens[i]
                        .0
                        .value
                        .strip_prefix("def_")
                        .and_then(Type::from_name);
                    let end = self
                        .find(i..range.end, &[Kind::Semicolon])
                        .unwrap_or(range.end);
                    if let Some(equals) = self.find(i..end, &[Kind::Equals]) {
                        self.expression(equals + 1..end, ty, depth + 1);
                    }
                    i = end + 1;
                }
                Kind::Semicolon | Kind::Else => i += 1,
                _ => {
                    let end = self
                        .find(i..range.end, &[Kind::Semicolon, Kind::LBrace])
                        .unwrap_or(range.end);
                    self.expression(i..end, None, depth + 1);
                    i = if end == i { i + 1 } else { end };
                }
            }
        }
    }

    fn arguments(
        &mut self,
        range: Range<usize>,
        types: &[Type],
        fallback: Option<Type>,
        depth: usize,
    ) {
        let mut start = range.start;
        let mut argument = 0;
        while start < range.end {
            let end = self
                .find(start..range.end, &[Kind::Comma])
                .unwrap_or(range.end);
            self.expression(
                start..end,
                types.get(argument).copied().or(fallback),
                depth + 1,
            );
            start = end + 1;
            argument += 1;
        }
    }

    fn inferred(&self, mut range: Range<usize>) -> Option<Type> {
        while !range.is_empty()
            && self.kind(range.start) == Kind::LParen
            && self.pairs[range.start] == Some(range.end - 1)
        {
            range = range.start + 1..range.end - 1;
        }
        let start = range.find(|&i| {
            !matches!(
                self.kind(i),
                Kind::LocalVar
                    | Kind::GameVar
                    | Kind::ConstantVar
                    | Kind::ScriptCall
                    | Kind::JumpCall
                    | Kind::Dot
            )
        })?;
        self.bindings[start].and_then(|symbol| symbol.value_type)
    }

    fn expression(&mut self, range: Range<usize>, expected: Option<Type>, depth: usize) {
        if depth > 64 || range.is_empty() {
            return;
        }
        if let Some(op) = self.find(range.clone(), &[Kind::LogicalAnd, Kind::LogicalOr]) {
            self.expression(range.start..op, None, depth + 1);
            self.expression(op + 1..range.end, None, depth + 1);
            return;
        }
        if let Some(op) = self.find(range.clone(), &[Kind::Equals, Kind::ComparisonOperator]) {
            let left = range.start..op;
            let right = op + 1..range.end;
            if self.find(left.clone(), &[Kind::Comma]).is_some() {
                let mut types = Vec::new();
                let mut start = left.start;
                while start < left.end {
                    let end = self
                        .find(start..left.end, &[Kind::Comma])
                        .unwrap_or(left.end);
                    types.push(self.inferred(start..end).unwrap_or(Type::Any));
                    start = end + 1;
                }
                self.arguments(right, &types, None, depth + 1);
                return;
            }
            let lhs = self.inferred(left.clone());
            let rhs = self.inferred(right.clone());
            self.expression(left, rhs, depth + 1);
            self.expression(right, lhs, depth + 1);
            return;
        }
        let mut i = range.start;
        while i < range.end {
            if self.kind(i) == Kind::LParen {
                let end = self.pairs[i].unwrap_or(range.end);
                self.expression(i + 1..end, expected, depth + 1);
                i = end + 1;
            } else if i + 1 < range.end && self.kind(i + 1) == Kind::LParen {
                let end = self.pairs[i + 1].unwrap_or(range.end);
                if let Some(symbol) = self.bindings[i] {
                    let types = symbol
                        .parameters
                        .iter()
                        .map(|(_, ty)| *ty)
                        .collect::<Vec<_>>();
                    self.arguments(i + 2..end, &types, None, depth + 1);
                } else {
                    self.expression(i + 2..end, expected, depth + 1);
                }
                i = end + 1;
            } else {
                self.expected[i] = expected;
                i += 1;
            }
        }
    }
}
