//! TypeChecker integration tests.
//!
//! Each test feeds a small RuneScript fragment through lex → parse →
//! script-registration → typecheck and asserts on the typechecker's
//! diagnostics. This locks in observable error behaviour so future
//! refactors of the typechecker can't silently change which scripts
//! are accepted or what error text users see.
//!
//! These are integration tests against the **public** lib API
//! (`Lexer`, `Parser`, `SymbolRegistry`, `TypeChecker`). They do not
//! exercise codegen; type errors are gated to the typechecker phase.
//!
//! Coverage groups:
//!   - happy path (clean script → no diagnostics)
//!   - trigger constraint checks (subject rules, params, returns)
//!   - local variable rules (redeclaration, undefined ref)
//!   - reference resolution (`~proc`, `@jump`, `^const`, `%gamevar`, command)
//!   - condition / switch validation
//!   - `*` suffix gated to command trigger

use rs_compiler::diagnostics::{Diagnostic, Severity};
use rs_compiler::lexer::Lexer;
use rs_compiler::parser::Parser;
use rs_compiler::symbol::SymbolRegistry;
use rs_compiler::typechecker::TypeChecker;
use rs_compiler::types::Type;
use std::path::PathBuf;

// ── Test harness ────────────────────────────────────────────────────

/// Result of running the typechecker against a snippet — we keep the
/// diagnostics by value so each test can poke at them however it likes.
struct CheckResult {
    diagnostics: Vec<Diagnostic>,
}

impl CheckResult {
    fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    fn error_count(&self) -> usize {
        self.errors().len()
    }

    fn has_error_containing(&self, needle: &str) -> bool {
        self.errors().iter().any(|d| d.message.contains(needle))
    }

    /// Pretty-print all diagnostics — used in assertion messages so a
    /// failing test points straight at what fired.
    fn dump(&self) -> String {
        self.diagnostics
            .iter()
            .map(|d| format!("[{:?}] {}", d.severity, d.message))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Default registry seeded with a minimal set of symbols — enough to
/// exercise typechecker paths that look up commands, entities, etc.
/// Tests that need more can call `seed_*` helpers below.
fn empty_registry() -> SymbolRegistry {
    SymbolRegistry::new()
}

/// Seed registry with a no-arg `mes` command (a stand-in for the
/// `mes(string)` engine command). `mes` is referenced widely in the
/// test fragments so this keeps test scripts readable.
fn seed_mes(reg: &mut SymbolRegistry) {
    reg.register_command("mes".into(), 1000, vec![Type::String], vec![]);
}

/// Lex, parse, register all scripts in the source, then run the
/// typechecker against the parsed file. Returns the typechecker's
/// diagnostics. Lexer / parser failures fail the test outright — those
/// have their own integration tests.
fn check_with_registry(src: &str, mut registry: SymbolRegistry) -> CheckResult {
    let path = PathBuf::from("test.rs2");
    let tokens = Lexer::new(src, &path)
        .tokenize()
        .expect("lex failed in typechecker test fixture");
    let file = Parser::new(tokens, &path)
        .parse()
        .expect("parse failed in typechecker test fixture");

    // Mirror main.rs Phase 2: register every script in the file so
    // proc/jump/script lookups resolve.
    for script in &file.scripts {
        if script.trigger == "command" {
            continue;
        }
        let param_types: Vec<Type> = script.params.iter().map(|p| p.param_type).collect();
        registry.register_script(
            script.name.clone(),
            script.trigger.clone(),
            param_types,
            script.return_types.clone(),
        );
    }

    let mut tc = TypeChecker::new(&registry);
    tc.check_file(&file, &path);
    CheckResult {
        diagnostics: tc.diagnostics.diagnostics().to_vec(),
    }
}

fn check(src: &str) -> CheckResult {
    check_with_registry(src, empty_registry())
}

// ── Happy path ──────────────────────────────────────────────────────

#[test]
fn empty_script_body_typechecks() {
    let res = check("[proc,noop]\n");
    assert_eq!(
        res.error_count(),
        0,
        "expected no errors, got:\n{}",
        res.dump()
    );
}

#[test]
fn simple_int_local_typechecks() {
    let res = check(
        "[proc,init_local]\n\
         def_int $x = 1;\n",
    );
    assert_eq!(res.error_count(), 0, "got:\n{}", res.dump());
}

#[test]
fn proc_with_typed_params_typechecks() {
    let res = check(
        "[proc,double](int $x)(int)\n\
         return($x);\n",
    );
    assert_eq!(res.error_count(), 0, "got:\n{}", res.dump());
}

// ── Local variable rules ────────────────────────────────────────────

#[test]
fn duplicate_param_name_errors() {
    let res = check("[proc,foo](int $a, int $a)\n");
    assert!(
        res.has_error_containing("$a") || res.has_error_containing("a"),
        "expected redeclaration error for $a, got:\n{}",
        res.dump()
    );
}

#[test]
fn undefined_local_reference_errors() {
    // `$missing` is never declared.
    let res = check(
        "[proc,oops]\n\
         def_int $x = $missing;\n",
    );
    assert!(
        res.has_error_containing("$missing")
            || res.has_error_containing("could not be resolved"),
        "expected unresolved-local error, got:\n{}",
        res.dump()
    );
}

// ── Reference resolution ────────────────────────────────────────────

#[test]
fn unresolved_proc_call_errors() {
    let res = check(
        "[proc,caller]\n\
         ~not_a_real_proc;\n",
    );
    assert!(
        res.has_error_containing("not_a_real_proc"),
        "expected unresolved-proc error, got:\n{}",
        res.dump()
    );
}

#[test]
fn unresolved_jump_call_errors() {
    let res = check(
        "[proc,caller]\n\
         @not_a_real_label;\n",
    );
    assert!(
        res.has_error_containing("not_a_real_label"),
        "expected unresolved-label error, got:\n{}",
        res.dump()
    );
}

#[test]
fn unresolved_constant_errors() {
    let res = check(
        "[proc,uses_const]\n\
         def_int $x = ^NOPE_NOT_A_CONST;\n",
    );
    assert!(
        res.has_error_containing("NOPE_NOT_A_CONST"),
        "expected unresolved-constant error, got:\n{}",
        res.dump()
    );
}

#[test]
fn unresolved_command_errors() {
    let res = check(
        "[proc,uses_cmd]\n\
         not_a_real_command;\n",
    );
    assert!(
        res.error_count() > 0,
        "expected at least one error, got:\n{}",
        res.dump()
    );
}

#[test]
fn proc_call_resolves_when_target_exists() {
    let res = check(
        "[proc,helper]\n\
         [proc,caller]\n\
         ~helper;\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "expected no errors when proc target exists, got:\n{}",
        res.dump()
    );
}

#[test]
fn jump_call_resolves_when_label_exists() {
    let res = check(
        "[label,trampoline]\n\
         [proc,caller]\n\
         @trampoline;\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "expected no errors when label target exists, got:\n{}",
        res.dump()
    );
}

#[test]
fn constant_reference_resolves_when_registered() {
    let mut registry = SymbolRegistry::new();
    registry.register_constant("MAX_HP".into(), Type::Int, Some(99), None);
    let res = check_with_registry(
        "[proc,uses_const]\n\
         def_int $x = ^MAX_HP;\n",
        registry,
    );
    assert_eq!(res.error_count(), 0, "got:\n{}", res.dump());
}

#[test]
fn command_reference_resolves_when_registered() {
    let mut registry = SymbolRegistry::new();
    seed_mes(&mut registry);
    let res = check_with_registry(
        "[proc,greet]\n\
         mes(\"hi\");\n",
        registry,
    );
    assert_eq!(res.error_count(), 0, "got:\n{}", res.dump());
}

// ── Trigger subject rules ───────────────────────────────────────────

#[test]
fn proc_with_global_subject_errors() {
    // `[proc,_]` — proc disallows global subject (only debugproc/command do).
    let res = check("[proc,_]\n");
    assert!(
        res.has_error_containing("global"),
        "expected SCRIPT_SUBJECT_NO_GLOBAL for [proc,_], got:\n{}",
        res.dump()
    );
}

#[test]
fn proc_with_category_subject_errors() {
    // `[proc,_some_cat]` — proc disallows category subjects.
    let res = check("[proc,_some_cat]\n");
    assert!(
        res.has_error_containing("category"),
        "expected SCRIPT_SUBJECT_NO_CATEGORY, got:\n{}",
        res.dump()
    );
}

#[test]
fn login_with_non_global_subject_errors() {
    // login only allows `_` subject.
    let res = check("[login,some_specific]\n");
    assert!(
        res.has_error_containing("global"),
        "expected SCRIPT_SUBJECT_ONLY_GLOBAL for [login,some_specific], got:\n{}",
        res.dump()
    );
}

#[test]
fn login_with_global_subject_typechecks() {
    let res = check("[login,_]\n");
    assert_eq!(
        res.error_count(),
        0,
        "[login,_] should be valid, got:\n{}",
        res.dump()
    );
}

#[test]
fn login_with_params_errors() {
    let res = check("[login,_](int $x)\n");
    assert!(
        res.has_error_containing("login")
            && res.has_error_containing("parameter"),
        "expected SCRIPT_TRIGGER_NO_PARAMETERS, got:\n{}",
        res.dump()
    );
}

#[test]
fn label_with_returns_errors() {
    // label disallows return values — only proc/logout do. Parser requires a
    // param list before a return list, so we add a throwaway `int $x` param.
    let res = check(
        "[label,foo](int $x)(int)\n\
         return(0);\n",
    );
    assert!(
        res.has_error_containing("label")
            && (res.has_error_containing("return") || res.has_error_containing("returns")),
        "expected SCRIPT_TRIGGER_NO_RETURNS for [label,foo](…)(int), got:\n{}",
        res.dump()
    );
}

#[test]
fn star_suffix_only_allowed_for_commands() {
    // `name*` syntax marks a vararg command; using it on a proc is an error.
    let res = check("[proc,foo*]\n");
    assert!(
        res.has_error_containing("'*'") || res.has_error_containing("commands"),
        "expected SCRIPT_COMMAND_ONLY, got:\n{}",
        res.dump()
    );
}

// ── Switch / condition checks ───────────────────────────────────────

// (Note: duplicate `default` labels are caught by the parser as a
// SyntaxError, not by the typechecker. That makes them a parser-test
// concern, not a typechecker concern; covering it here would just test
// that we never reach the typechecker.)

#[test]
fn case_outside_switch_errors() {
    // A `case` statement outside a `switch_*` block is an orphan.
    // Parser may or may not surface this; the typechecker definitely should.
    let res = check(
        "[proc,bad]\n\
         case 1 : return;\n",
    );
    // We don't assert on a specific message — orphan case can surface
    // through several error paths. Just confirm something fires.
    assert!(
        res.error_count() > 0,
        "expected at least one error for orphan case, got:\n{}",
        res.dump()
    );
}

// ── Assignment type rules ───────────────────────────────────────────

#[test]
fn assigning_string_to_int_errors() {
    let res = check(
        "[proc,bad_assign]\n\
         def_int $x = \"not a number\";\n",
    );
    assert!(
        res.has_error_containing("Type mismatch")
            || res.has_error_containing("int")
            || res.has_error_containing("string"),
        "expected type-mismatch error, got:\n{}",
        res.dump()
    );
}

#[test]
fn return_wrong_type_errors() {
    // Parser requires a param list before a return list — add a throwaway
    // param so the script parses; the typechecker should still catch the
    // string-vs-int return mismatch.
    let res = check(
        "[proc,wrong_return](int $x)(int)\n\
         return(\"not an int\");\n",
    );
    assert!(
        res.error_count() > 0,
        "expected error returning string from int proc, got:\n{}",
        res.dump()
    );
}

// ── Diagnostic shape ────────────────────────────────────────────────

#[test]
fn errors_carry_source_line() {
    // The undefined `$missing` reference is on line 2.
    let res = check(
        "[proc,oops]\n\
         def_int $x = $missing;\n",
    );
    let err = res
        .errors()
        .into_iter()
        .find(|d| d.message.contains("$missing") || d.message.contains("missing"))
        .expect("expected an error mentioning $missing");
    assert_eq!(
        err.line, 2,
        "expected error on line 2, got line {}: {}",
        err.line, err.message
    );
}

#[test]
fn errors_use_typechecking_phase() {
    use rs_compiler::diagnostics::Phase;
    let res = check(
        "[proc,oops]\n\
         ~not_a_real_proc;\n",
    );
    for err in res.errors() {
        assert_eq!(
            err.phase,
            Phase::TypeChecking,
            "every typechecker diagnostic should use Phase::TypeChecking, got {:?} for: {}",
            err.phase,
            err.message
        );
    }
}
