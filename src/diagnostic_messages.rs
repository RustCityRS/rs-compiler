/// Diagnostic message templates matching the reference RuneScriptTS compiler.
/// Use `fmt` to substitute `%s` placeholders with arguments.
// Internal compiler errors
pub const UNSUPPORTED_SYMBOLTYPE_TO_TYPE: &str =
    "Internal compiler error: Unsupported SymbolType -> Type conversion: %s";
pub const CASE_WITHOUT_SWITCH: &str =
    "Internal compiler error: Case without switch statement as parent.";
pub const RETURN_ORPHAN: &str =
    "Internal compiler error: Orphaned `return` statement, no parent `script` node found.";
pub const TRIGGER_TYPE_NOT_FOUND: &str =
    "Internal compiler error: The trigger '%s' has no declaration.";

// Code gen internal compiler errors
pub const SYMBOL_IS_NULL: &str =
    "Internal compiler error: Symbol has not been defined for the node.";
pub const TYPE_HAS_NO_BASETYPE: &str =
    "Internal compiler error: Type has no defined base type: %s.";
pub const TYPE_HAS_NO_DEFAULT: &str =
    "Internal compiler error: Return type '%s' has no defined default value.";
pub const INVALID_CONDITION: &str =
    "Internal compiler error: %s is not a supported expression type for conditions.";
pub const NULL_CONSTANT: &str = "Internal compiler error: %s evaluated to 'null' constant value.";
pub const EXPRESSION_NO_SUBEXPR: &str = "Internal compiler error: No sub expression node.";

// Node type agnostic messages
pub const GENERIC_INVALID_TYPE: &str = "'%s' is not a valid type.";
pub const GENERIC_TYPE_MISMATCH: &str = "Type mismatch: '%s' was given but '%s' was expected.";
pub const GENERIC_UNRESOLVED_SYMBOL: &str = "'%s' could not be resolved to a symbol.";
pub const ARITHMETIC_INVALID_TYPE: &str =
    "Type mismatch: '%s' was given but 'int' or 'long' was expected.";

// Script node specific
pub const SCRIPT_REDECLARATION: &str = "[%s,%s] is already defined.";
pub const SCRIPT_LOCAL_REDECLARATION: &str = "'$%s' is already defined.";
pub const SCRIPT_TRIGGER_INVALID: &str = "'%s' is not a valid trigger type.";
pub const SCRIPT_COMMAND_ONLY: &str = "Using a '*' is only allowed for commands.";
pub const SCRIPT_TRIGGER_NO_PARAMETERS: &str =
    "The trigger type '%s' is not allowed to have parameters defined.";
pub const SCRIPT_TRIGGER_EXPECTED_PARAMETERS: &str =
    "The trigger type '%s' is expected to accept (%s).";
pub const SCRIPT_TRIGGER_NO_RETURNS: &str =
    "The trigger type '%s' is not allowed to return values.";
pub const SCRIPT_TRIGGER_EXPECTED_RETURNS: &str =
    "The trigger type '%s' is expected to return (%s).";
pub const SCRIPT_SUBJECT_ONLY_GLOBAL: &str = "Trigger '%s' only allows global subjects.";
pub const SCRIPT_SUBJECT_NO_GLOBAL: &str = "Trigger '%s' does not allow global subjects.";
pub const SCRIPT_SUBJECT_NO_CATEGORY: &str = "Trigger '%s' does not allow category subjects.";
pub const SCRIPT_SUBJECT_NO_SPACES: &str = "Trigger '%s' does not allow spaces in subjects.";

// Switch statement node specific
pub const SWITCH_INVALID_TYPE: &str = "'%s' is not allowed within a switch statement.";
pub const SWITCH_DUPLICATE_DEFAULT: &str = "Duplicate default label.";
pub const SWITCH_CASE_NOT_CONSTANT: &str = "Switch case value is not a constant expression.";

// Assignment statement node specific
pub const ASSIGN_MULTI_ARRAY: &str = "Arrays are not allowed in multi-assignment statements.";

// Expression statement node specific
pub const EXPRESSION_STATEMENT_NO_SIDE_EFFECT: &str = "Value is discarded.";

// Condition expression specific
pub const CONDITION_INVALID_NODE_TYPE: &str =
    "Conditions are only allowed to be binary expressions.";
pub const CONDITION_NOT_VALID: &str = "Condition is not valid.";

// Binary expression specific
pub const BINOP_INVALID_TYPES: &str = "Operator '%s' cannot be applied to '%s', '%s'.";
pub const BINOP_TUPLE_TYPE: &str =
    "%s side of binary expressions can only have one type but has '%s'.";

// Call expression specific
pub const COMMAND_REFERENCE_UNRESOLVED: &str = "'%s' cannot be resolved to a command.";
pub const COMMAND_BARE_REQUIRES_ARGS: &str = "Command '%s' takes arguments but is used here as a bare value, which compiles to its opcode (an integer), not its result. Did you mean to call it, e.g. `%s(...)`?";
pub const COMMAND_NOARGS_EXPECTED: &str = "'%s' is expected to have no arguments but has '%s'.";
pub const PROC_REFERENCE_UNRESOLVED: &str = "'~%s' cannot be resolved to a proc.";
pub const PROC_NOARGS_EXPECTED: &str = "'~%s' is expected to have no arguments but has '%s'.";
pub const JUMP_REFERENCE_UNRESOLVED: &str = "'@%s' cannot be resolved to a label.";
pub const JUMP_NOARGS_EXPECTED: &str = "'@%s' is expected to have no arguments but has '%s'.";
pub const CLIENTSCRIPT_REFERENCE_UNRESOLVED: &str = "'%s' cannot be resolved to a clientscript.";
pub const CLIENTSCRIPT_NOARGS_EXPECTED: &str =
    "'%s' is expected to have no arguments but has '%s'.";
pub const HOOK_TRANSMIT_LIST_UNEXPECTED: &str = "Unexpected hook transmit list.";

// Local variable specific
pub const LOCAL_DECLARATION_INVALID_TYPE: &str = "'%s' is not allowed to be declared as a type.";
pub const LOCAL_PARAMETER_INVALID_TYPE: &str = "'%s' is not allowed to be used as a parameter.";
pub const LOCAL_REFERENCE_UNRESOLVED: &str = "'$%s' cannot be resolved to a local variable.";
pub const LOCAL_REFERENCE_NOT_ARRAY: &str =
    "Access of indexed value of non-array type variable '$%s'.";
pub const LOCAL_ARRAY_INVALID_TYPE: &str = "'%s' is not allowed to be used as an array.";
pub const LOCAL_ARRAY_REFERENCE_NOINDEX: &str =
    "'$%s' is a reference to an array variable without specifying the index.";

// Game var specific
pub const GAME_REFERENCE_UNRESOLVED: &str = "'%%s' cannot be resolved to a game variable.";

// Constant variable specific
pub const CONSTANT_REFERENCE_UNRESOLVED: &str = "'^%s' cannot be resolved to a constant.";
pub const CONSTANT_CYCLIC_REF: &str = "Cyclic constant references are not permitted: %s.";
pub const CONSTANT_UNKNOWN_TYPE: &str = "Unable to infer type for '^%s'.";
pub const CONSTANT_PARSE_ERROR: &str = "Unable to parse constant value of '%s' into type '%s'.";
pub const CONSTANT_NONCONSTANT: &str =
    "Constant value of '%s' evaluated to a non-constant expression.";

// Pointer checking.
//
// Two vocabularies:
//
// 1. Engine-tracked pointers (active_player, p_active_player, active_npc,
//    active_loc, active_obj and their secondary variants). These are
//    enforced at runtime by `ScriptState.pointerCheck`; a missing
//    requirement is a real crash. Strong wording ("uninitialized" /
//    "corrupted") is appropriate.
//
// 2. Static-only pointers (last_*, find_*). These are NOT tracked at
//    runtime — `last_useitem`, `last_com`, etc. are plain fields on the
//    player/script state, and no engine opcode invalidates them based on
//    `p_delay` or subroutine boundaries. A warning here flags a stale
//    reference / brittle ordering, not a crash risk. Softer wording
//    avoids overstating the problem.
pub const POINTER_UNINITIALIZED: &str = "Attempt to access uninitialized pointer %s.";
pub const POINTER_CORRUPTED: &str = "Attempt to access corrupted pointer %s.";
pub const POINTER_CORRUPTED_LOC: &str = "%s corrupted here.";
pub const POINTER_REQUIRED_LOC: &str = "%s required here.";

// Softer wording for static-only (last_*, find_*) pointers. These scripts
// compile and run; the warning surfaces code that is brittle to reorder.
pub const POINTER_STALE: &str = "Possibly stale `%s` reference — value may not reflect the caller's intent after an intervening delay or subroutine.";
pub const POINTER_STALE_UNINIT: &str = "`%s` is not available in this trigger context; the engine gates this read on the entry trigger and will reject it at runtime.";
pub const POINTER_READ_HERE: &str = "`%s` is read here.";
pub const POINTER_SUPERSEDED_HERE: &str = "`%s` may be superseded on this path (e.g. by `p_delay` or a subroutine that delays); the runtime still returns the original value, but relying on it is brittle.";

// Entity reference lints — fired when an identifier or string literal is
// used at a slot with an entity-typed hint but fails to resolve against
// the registry's typed entity table.
pub const UNRESOLVED_ENTITY_REF: &str = "`%s` does not resolve to a known `%s` — the compiler will fall back to a sentinel (`-1` for bare identifiers, raw string for string literals), which the engine cannot use as an entity id.";

// Style lint — string literal whose resolved name is a valid bare
// identifier (no spaces or other quote-forcing chars). Prefer the bare
// form for clarity.
pub const PREFER_BARE_IDENT: &str = "`\"%s\"` resolves to a `%s` entity; the bare identifier form is preferred when the name has no spaces.";

pub const TEST_PROC_FROM_PRODUCTION: &str = "Cannot call test proc '~%s' from production code. Move the call below #testscript or move the proc above it.";
pub const TEST_LABEL_FROM_PRODUCTION: &str = "Cannot jump to test label '@%s' from production code. Move the jump below #testscript or move the label above it.";
pub const TEST_COMMAND_FROM_PRODUCTION: &str = "Cannot use test command '%s' from production code. Test commands are only available below #testscript.";

/// Format a diagnostic message template by replacing `%s` placeholders with arguments.
pub fn fmt(template: &str, args: &[&str]) -> String {
    let mut result = template.to_string();
    for arg in args {
        if let Some(pos) = result.find("%s") {
            result.replace_range(pos..pos + 2, arg);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_no_args() {
        assert_eq!(fmt("no placeholders here", &[]), "no placeholders here");
    }

    #[test]
    fn fmt_single_arg() {
        assert_eq!(fmt("hello %s", &["world"]), "hello world");
    }

    #[test]
    fn fmt_multiple_args() {
        assert_eq!(
            fmt(
                "Type mismatch: '%s' was given but '%s' was expected.",
                &["int", "string"]
            ),
            "Type mismatch: 'int' was given but 'string' was expected."
        );
    }

    #[test]
    fn fmt_game_var_sigil() {
        assert_eq!(
            fmt(GAME_REFERENCE_UNRESOLVED, &["testing_maine"]),
            "'%testing_maine' cannot be resolved to a game variable."
        );
    }

    #[test]
    fn fmt_local_var_sigil() {
        assert_eq!(
            fmt(LOCAL_REFERENCE_UNRESOLVED, &["myvar"]),
            "'$myvar' cannot be resolved to a local variable."
        );
    }

    #[test]
    fn fmt_proc_sigil() {
        assert_eq!(
            fmt(PROC_REFERENCE_UNRESOLVED, &["myproc"]),
            "'~myproc' cannot be resolved to a proc."
        );
    }

    #[test]
    fn fmt_jump_sigil() {
        assert_eq!(
            fmt(JUMP_REFERENCE_UNRESOLVED, &["mylabel"]),
            "'@mylabel' cannot be resolved to a label."
        );
    }

    #[test]
    fn fmt_constant_sigil() {
        assert_eq!(
            fmt(CONSTANT_REFERENCE_UNRESOLVED, &["myconst"]),
            "'^myconst' cannot be resolved to a constant."
        );
    }

    #[test]
    fn fmt_internal_errors() {
        assert_eq!(
            fmt(UNSUPPORTED_SYMBOLTYPE_TO_TYPE, &["Foo"]),
            "Internal compiler error: Unsupported SymbolType -> Type conversion: Foo"
        );
        assert_eq!(
            CASE_WITHOUT_SWITCH,
            "Internal compiler error: Case without switch statement as parent."
        );
        assert_eq!(
            RETURN_ORPHAN,
            "Internal compiler error: Orphaned `return` statement, no parent `script` node found."
        );
        assert_eq!(
            fmt(TRIGGER_TYPE_NOT_FOUND, &["clientscript"]),
            "Internal compiler error: The trigger 'clientscript' has no declaration."
        );
        assert_eq!(
            SYMBOL_IS_NULL,
            "Internal compiler error: Symbol has not been defined for the node."
        );
        assert_eq!(
            fmt(TYPE_HAS_NO_BASETYPE, &["void"]),
            "Internal compiler error: Type has no defined base type: void."
        );
        assert_eq!(
            fmt(TYPE_HAS_NO_DEFAULT, &["void"]),
            "Internal compiler error: Return type 'void' has no defined default value."
        );
        assert_eq!(
            fmt(INVALID_CONDITION, &["Calc"]),
            "Internal compiler error: Calc is not a supported expression type for conditions."
        );
        assert_eq!(
            fmt(NULL_CONSTANT, &["^myconst"]),
            "Internal compiler error: ^myconst evaluated to 'null' constant value."
        );
        assert_eq!(
            EXPRESSION_NO_SUBEXPR,
            "Internal compiler error: No sub expression node."
        );
    }

    #[test]
    fn fmt_generic_messages() {
        assert_eq!(
            fmt(GENERIC_UNRESOLVED_SYMBOL, &["mystery_item"]),
            "'mystery_item' could not be resolved to a symbol."
        );
    }

    #[test]
    fn fmt_script_messages() {
        assert_eq!(
            fmt(SCRIPT_REDECLARATION, &["proc", "myproc"]),
            "[proc,myproc] is already defined."
        );
        assert_eq!(
            fmt(SCRIPT_LOCAL_REDECLARATION, &["count"]),
            "'$count' is already defined."
        );
        assert_eq!(
            fmt(SCRIPT_TRIGGER_INVALID, &["badtrig"]),
            "'badtrig' is not a valid trigger type."
        );
        assert_eq!(
            SCRIPT_COMMAND_ONLY,
            "Using a '*' is only allowed for commands."
        );
        assert_eq!(
            fmt(SCRIPT_TRIGGER_NO_PARAMETERS, &["debugproc"]),
            "The trigger type 'debugproc' is not allowed to have parameters defined."
        );
        assert_eq!(
            fmt(SCRIPT_TRIGGER_EXPECTED_PARAMETERS, &["queue", "int"]),
            "The trigger type 'queue' is expected to accept (int)."
        );
        assert_eq!(
            fmt(SCRIPT_TRIGGER_NO_RETURNS, &["label"]),
            "The trigger type 'label' is not allowed to return values."
        );
        assert_eq!(
            fmt(SCRIPT_TRIGGER_EXPECTED_RETURNS, &["proc", "int"]),
            "The trigger type 'proc' is expected to return (int)."
        );
        assert_eq!(
            fmt(SCRIPT_SUBJECT_ONLY_GLOBAL, &["login"]),
            "Trigger 'login' only allows global subjects."
        );
        assert_eq!(
            fmt(SCRIPT_SUBJECT_NO_GLOBAL, &["proc"]),
            "Trigger 'proc' does not allow global subjects."
        );
        assert_eq!(
            fmt(SCRIPT_SUBJECT_NO_CATEGORY, &["proc"]),
            "Trigger 'proc' does not allow category subjects."
        );
        assert_eq!(
            fmt(SCRIPT_SUBJECT_NO_SPACES, &["proc"]),
            "Trigger 'proc' does not allow spaces in subjects."
        );
    }

    #[test]
    fn fmt_switch_messages() {
        assert_eq!(
            fmt(SWITCH_INVALID_TYPE, &["string"]),
            "'string' is not allowed within a switch statement."
        );
        assert_eq!(SWITCH_DUPLICATE_DEFAULT, "Duplicate default label.");
        assert_eq!(
            SWITCH_CASE_NOT_CONSTANT,
            "Switch case value is not a constant expression."
        );
    }

    #[test]
    fn fmt_assignment_messages() {
        assert_eq!(
            ASSIGN_MULTI_ARRAY,
            "Arrays are not allowed in multi-assignment statements."
        );
    }

    #[test]
    fn fmt_condition_messages() {
        assert_eq!(
            CONDITION_INVALID_NODE_TYPE,
            "Conditions are only allowed to be binary expressions."
        );
        assert_eq!(CONDITION_NOT_VALID, "Condition is not valid.");
    }

    #[test]
    fn fmt_binary_messages() {
        assert_eq!(
            fmt(BINOP_INVALID_TYPES, &["+", "string", "int"]),
            "Operator '+' cannot be applied to 'string', 'int'."
        );
        assert_eq!(
            fmt(BINOP_TUPLE_TYPE, &["Left", "int, string"]),
            "Left side of binary expressions can only have one type but has 'int, string'."
        );
    }

    #[test]
    fn fmt_call_messages() {
        assert_eq!(
            fmt(COMMAND_REFERENCE_UNRESOLVED, &["badcmd"]),
            "'badcmd' cannot be resolved to a command."
        );
        assert_eq!(
            fmt(COMMAND_NOARGS_EXPECTED, &["mes", "int"]),
            "'mes' is expected to have no arguments but has 'int'."
        );
        assert_eq!(
            fmt(CLIENTSCRIPT_REFERENCE_UNRESOLVED, &["badcs"]),
            "'badcs' cannot be resolved to a clientscript."
        );
        assert_eq!(
            fmt(CLIENTSCRIPT_NOARGS_EXPECTED, &["cs", "int"]),
            "'cs' is expected to have no arguments but has 'int'."
        );
        assert_eq!(
            HOOK_TRANSMIT_LIST_UNEXPECTED,
            "Unexpected hook transmit list."
        );
    }

    #[test]
    fn fmt_local_messages() {
        assert_eq!(
            fmt(LOCAL_DECLARATION_INVALID_TYPE, &["void"]),
            "'void' is not allowed to be declared as a type."
        );
        assert_eq!(
            fmt(LOCAL_PARAMETER_INVALID_TYPE, &["void"]),
            "'void' is not allowed to be used as a parameter."
        );
        assert_eq!(
            fmt(LOCAL_ARRAY_INVALID_TYPE, &["string"]),
            "'string' is not allowed to be used as an array."
        );
        assert_eq!(
            fmt(LOCAL_ARRAY_REFERENCE_NOINDEX, &["arr"]),
            "'$arr' is a reference to an array variable without specifying the index."
        );
    }

    #[test]
    fn fmt_constant_messages() {
        assert_eq!(
            fmt(CONSTANT_CYCLIC_REF, &["^a -> ^b -> ^a"]),
            "Cyclic constant references are not permitted: ^a -> ^b -> ^a."
        );
        assert_eq!(
            fmt(CONSTANT_UNKNOWN_TYPE, &["myconst"]),
            "Unable to infer type for '^myconst'."
        );
        assert_eq!(
            fmt(CONSTANT_PARSE_ERROR, &["abc", "int"]),
            "Unable to parse constant value of 'abc' into type 'int'."
        );
        assert_eq!(
            fmt(CONSTANT_NONCONSTANT, &["~proc_call"]),
            "Constant value of '~proc_call' evaluated to a non-constant expression."
        );
    }

    #[test]
    fn fmt_pointer_messages() {
        assert_eq!(
            fmt(POINTER_UNINITIALIZED, &["active_npc"]),
            "Attempt to access uninitialized pointer active_npc."
        );
        assert_eq!(
            fmt(POINTER_CORRUPTED, &["active_npc"]),
            "Attempt to access corrupted pointer active_npc."
        );
        assert_eq!(
            fmt(POINTER_CORRUPTED_LOC, &["active_npc"]),
            "active_npc corrupted here."
        );
        assert_eq!(
            fmt(POINTER_REQUIRED_LOC, &["active_npc"]),
            "active_npc required here."
        );
    }
}
