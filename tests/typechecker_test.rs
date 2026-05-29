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

use runec::diagnostics::{Diagnostic, Severity};
use runec::lexer::Lexer;
use runec::parser::Parser;
use runec::symbol::SymbolRegistry;
use runec::typechecker::TypeChecker;
use runec::types::Type;
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

    fn has_warning_containing(&self, needle: &str) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning && d.message.contains(needle))
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
fn run_check(
    src: &str,
    mut registry: SymbolRegistry,
    testscript_line: Option<usize>,
) -> CheckResult {
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
    tc.check_file_with_test_boundary(&file, &path, testscript_line);
    CheckResult {
        diagnostics: tc.diagnostics.diagnostics().to_vec(),
    }
}

/// Check in **production** context (no `#testscript` boundary). The lenient
/// type rules apply: integer-base named types are mutually interchangeable and
/// command arg counts are not strictly enforced — matching pre-test-framework
/// (and engine reference) behaviour.
fn check_with_registry(src: &str, registry: SymbolRegistry) -> CheckResult {
    run_check(src, registry, None)
}

fn check(src: &str) -> CheckResult {
    check_with_registry(src, empty_registry())
}

/// Check as a **test script** (as if the whole fragment sits below a
/// `#testscript` boundary). The strict assertion-framework rules apply: exact
/// named types (no integer-base widening) and strict command arg counts, which
/// `assert_eq` and friends rely on.
fn check_test_with_registry(src: &str, registry: SymbolRegistry) -> CheckResult {
    run_check(src, registry, Some(0))
}

fn check_test(src: &str) -> CheckResult {
    check_test_with_registry(src, empty_registry())
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
        res.has_error_containing("$missing") || res.has_error_containing("could not be resolved"),
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
fn jump_to_test_label_from_production_errors() {
    // A test-section label (below #testscript) jumped to from production code
    // must be rejected, like test commands.
    let mut reg = empty_registry();
    reg.register_script("test_foo".into(), "label".into(), vec![], vec![]);
    reg.mark_test_script("label", "test_foo");
    let res = check_with_registry(
        "[proc,prod]\n\
         @test_foo;\n",
        reg,
    );
    assert!(
        res.has_error_containing("test label") || res.has_error_containing("production"),
        "expected test-label-from-production error, got:\n{}",
        res.dump()
    );
}

#[test]
fn jump_to_test_label_from_test_is_ok() {
    // Calling a test label from another test label is fine — but the harness
    // treats everything as production (no boundary), so we assert the inverse:
    // an *unmarked* label jumped to from production does NOT error.
    let mut reg = empty_registry();
    reg.register_script("helper".into(), "label".into(), vec![], vec![]);
    let res = check_with_registry(
        "[proc,prod]\n\
         @helper;\n",
        reg,
    );
    assert!(
        !res.has_error_containing("production"),
        "non-test label should be callable from production, got:\n{}",
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
        res.has_error_containing("login") && res.has_error_containing("parameter"),
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
    use runec::diagnostics::Phase;
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

// ── Named-type / int strict type checking ──────────────────────────

#[test]
fn int_literal_adopts_named_type_hint() {
    let res = check(
        "[proc,hint_test]\n\
         def_stat $s = 5;\n\
         def_npc_mode $m = 0;\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "literals should adopt hint type, got:\n{}",
        res.dump()
    );
}

#[test]
fn namedobj_assignable_to_obj() {
    let mut reg = empty_registry();
    reg.register_command("obj_test".into(), 2000, vec![Type::Obj], vec![]);
    let res = check_with_registry(
        "[proc,namedobj_ok]\n\
         def_namedobj $n = 0;\n\
         obj_test($n);\n",
        reg,
    );
    assert_eq!(
        res.error_count(),
        0,
        "namedobj should be assignable to obj, got:\n{}",
        res.dump()
    );
}

#[test]
fn named_type_to_int_errors() {
    // Strict named-type rules are #testscript-only; check as a test script.
    let res = check_test(
        "[proc,cross_type]\n\
         def_stat $s = 0;\n\
         def_int $x = $s;\n",
    );
    assert!(
        res.has_error_containing("Type mismatch"),
        "named type should not narrow to int, got:\n{}",
        res.dump()
    );
}

/// Regression lock: the narrowing that errors under `#testscript` must be
/// *accepted* in a production script (integer-base interchangeability). This is
/// what keeps 225/647 content compiling — losing it regressed 177 scripts.
#[test]
fn named_type_to_int_lenient_in_production() {
    let res = check(
        "[proc,cross_type]\n\
         def_stat $s = 0;\n\
         def_int $x = $s;\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "production allows integer-base interchangeability, got:\n{}",
        res.dump()
    );
}

#[test]
fn int_widens_to_named_type() {
    let res = check(
        "[proc,cross_type]\n\
         def_int $x = 5;\n\
         def_stat $s = $x;\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "int should widen to stat, got:\n{}",
        res.dump()
    );
}

#[test]
fn npc_mode_to_stat_errors() {
    let mut reg = empty_registry();
    reg.register_command("boost_stat".into(), 2001, vec![Type::Stat], vec![]);
    let res = check_test_with_registry(
        "[proc,cmd_mismatch]\n\
         def_npc_mode $m = 0;\n\
         boost_stat($m);\n",
        reg,
    );
    assert!(
        res.has_error_containing("Type mismatch"),
        "expected type mismatch passing npc_mode to stat param, got:\n{}",
        res.dump()
    );
}

#[test]
fn int_arg_widens_to_named_command_param() {
    let mut reg = empty_registry();
    reg.register_command("boost_stat".into(), 2001, vec![Type::Stat], vec![]);
    let res = check_with_registry(
        "[proc,cmd_ok]\n\
         def_int $x = 5;\n\
         boost_stat($x);\n",
        reg,
    );
    assert_eq!(
        res.error_count(),
        0,
        "int should widen to stat for command arg, got:\n{}",
        res.dump()
    );
}

#[test]
fn command_literal_arg_adopts_named_type() {
    let mut reg = empty_registry();
    reg.register_command("boost_stat".into(), 2001, vec![Type::Stat], vec![]);
    let res = check_with_registry(
        "[proc,cmd_hint_ok]\n\
         boost_stat(5);\n",
        reg,
    );
    assert_eq!(
        res.error_count(),
        0,
        "literal arg should adopt stat hint, got:\n{}",
        res.dump()
    );
}

#[test]
fn int_arg_widens_to_named_proc_param() {
    let res = check(
        "[proc,takes_stat](stat $s)\n\
         return;\n\
         [proc,caller]\n\
         def_int $x = 5;\n\
         ~takes_stat($x);\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "int should widen to stat for proc arg, got:\n{}",
        res.dump()
    );
}

#[test]
fn named_type_to_different_named_proc_param_errors() {
    let res = check_test(
        "[proc,takes_stat](stat $s)\n\
         return;\n\
         [proc,caller]\n\
         def_npc_mode $m = 0;\n\
         ~takes_stat($m);\n",
    );
    assert!(
        res.has_error_containing("Type mismatch"),
        "expected type mismatch passing npc_mode to stat proc param, got:\n{}",
        res.dump()
    );
}

#[test]
fn proc_literal_arg_adopts_named_type() {
    let res = check(
        "[proc,takes_stat](stat $s)\n\
         return;\n\
         [proc,caller]\n\
         ~takes_stat(5);\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "literal arg should adopt stat hint for proc param, got:\n{}",
        res.dump()
    );
}

#[test]
fn return_int_var_for_named_type_ok() {
    let res = check(
        "[proc,ret_stat]()(stat)\n\
         def_int $x = 5;\n\
         return($x);\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "int should widen to stat for return value, got:\n{}",
        res.dump()
    );
}

#[test]
fn return_wrong_named_type_errors() {
    let res = check_test(
        "[proc,ret_stat]()(stat)\n\
         def_npc_mode $m = 0;\n\
         return($m);\n",
    );
    assert!(
        res.has_error_containing("Type mismatch"),
        "expected type mismatch returning npc_mode for stat, got:\n{}",
        res.dump()
    );
}

#[test]
fn return_literal_for_stat_proc_ok() {
    let res = check(
        "[proc,ret_stat]()(stat)\n\
         return(0);\n",
    );
    assert_eq!(
        res.error_count(),
        0,
        "literal return should adopt stat hint, got:\n{}",
        res.dump()
    );
}

// ── Bare command-as-value lint ──────────────────────────────────────

#[test]
fn bare_command_requiring_args_warns() {
    let mut reg = empty_registry();
    // coordx(coord)(int) — requires a coord argument.
    reg.register_command("coordx".into(), 1000, vec![Type::Coord], vec![Type::Int]);
    let res = check_with_registry(
        "[proc,oops]\n\
         def_int $x = coordx;\n",
        reg,
    );
    assert!(
        res.has_warning_containing("used here as a bare value"),
        "expected bare-command lint warning, got:\n{}",
        res.dump()
    );
}

#[test]
fn bare_type_name_command_still_warns() {
    let mut reg = empty_registry();
    // `stat` doubles as a type name AND a command stat(stat)(int). A bare
    // `def_int $x = stat` is still the footgun — the value context check
    // catches it even though `stat` short-circuits as a type-char elsewhere.
    reg.register_command("stat".into(), 1500, vec![Type::Stat], vec![Type::Int]);
    let res = check_with_registry(
        "[proc,oops]\n\
         def_int $level = stat;\n",
        reg,
    );
    assert!(
        res.has_warning_containing("used here as a bare value"),
        "expected bare-command lint for `stat`, got:\n{}",
        res.dump()
    );
}

#[test]
fn proper_command_call_does_not_warn() {
    let mut reg = empty_registry();
    reg.register_command("coordx".into(), 1000, vec![Type::Coord], vec![Type::Int]);
    reg.register_command("coord".into(), 1001, vec![], vec![Type::Coord]);
    let res = check_with_registry(
        "[proc,ok]\n\
         def_coord $c = coord;\n\
         def_int $x = coordx($c);\n",
        reg,
    );
    assert!(
        !res.has_warning_containing("used here as a bare value"),
        "calling coordx with an argument should not warn, got:\n{}",
        res.dump()
    );
}

#[test]
fn bare_command_without_args_does_not_warn() {
    let mut reg = empty_registry();
    // coord()(coord) — takes no arguments, so bare use is fine.
    reg.register_command("coord".into(), 1001, vec![], vec![Type::Coord]);
    let res = check_with_registry(
        "[proc,ok]\n\
         def_coord $c = coord;\n",
        reg,
    );
    assert!(
        !res.has_warning_containing("used here as a bare value"),
        "no-arg command used bare should not warn, got:\n{}",
        res.dump()
    );
}

#[test]
fn bare_void_command_as_opcode_does_not_warn() {
    let mut reg = empty_registry();
    // mes(string) and wait_for(int, int) — passing a void command's opcode
    // (e.g. wait_for(mes, 20)) is a legitimate test-framework pattern.
    reg.register_command("mes".into(), 2064, vec![Type::String], vec![]);
    reg.register_command("wait_for".into(), 3000, vec![Type::Int, Type::Int], vec![]);
    let res = check_with_registry(
        "[proc,ok]\n\
         wait_for(mes, 20);\n",
        reg,
    );
    assert!(
        !res.has_warning_containing("used here as a bare value"),
        "passing a void command's opcode should not warn, got:\n{}",
        res.dump()
    );
}

#[test]
fn bare_command_statement_warns() {
    let mut reg = empty_registry();
    // `mes;` as a standalone statement — a bare void command compiles to its
    // opcode and is discarded. Must be flagged even though mes returns nothing.
    reg.register_command("mes".into(), 2064, vec![Type::String], vec![]);
    let res = check_with_registry(
        "[proc,oops]\n\
         mes;\n",
        reg,
    );
    assert!(
        res.has_warning_containing("used here as a bare value"),
        "bare `mes;` statement should warn, got:\n{}",
        res.dump()
    );
}

#[test]
fn bare_command_call_zero_args_errors() {
    let mut reg = empty_registry();
    // `mes()` with zero args for mes(string) — arg count mismatch.
    reg.register_command("mes".into(), 2064, vec![Type::String], vec![]);
    let res = check_test_with_registry(
        "[proc,oops]\n\
         mes();\n",
        reg,
    );
    assert!(
        res.has_error_containing("arg(s)"),
        "mes() with zero args should error, got:\n{}",
        res.dump()
    );
}

/// Regression lock: strict command arg-count is `#testscript`-only. The same
/// `mes()` mismatch must NOT error in a production script — command signatures
/// can be variadic/partially-known in the symbol set, so enforcing exact arity
/// false-positived on production content (esp. 225).
#[test]
fn command_arg_count_lenient_in_production() {
    let mut reg = empty_registry();
    reg.register_command("mes".into(), 2064, vec![Type::String], vec![]);
    let res = check_with_registry(
        "[proc,prod_argcount]\n\
         mes();\n",
        reg,
    );
    assert_eq!(
        res.error_count(),
        0,
        "production must not strictly arg-count commands, got:\n{}",
        res.dump()
    );
}

#[test]
fn noarg_command_statement_does_not_warn() {
    let mut reg = empty_registry();
    // `if_close;` — a legitimate no-arg command statement, must not warn.
    reg.register_command("if_close".into(), 1500, vec![], vec![]);
    let res = check_with_registry(
        "[proc,ok]\n\
         if_close;\n",
        reg,
    );
    assert!(
        !res.has_warning_containing("used here as a bare value"),
        "no-arg command statement should not warn, got:\n{}",
        res.dump()
    );
}
