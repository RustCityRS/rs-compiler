use runec::lexer::Lexer;
use runec::parser::Parser;
use std::path::PathBuf;

fn parse_source(source: &str) -> runec::parser::ScriptFile {
    let path = PathBuf::from("test.rs2");
    let tokens = Lexer::new(source, &path).tokenize().unwrap();
    let mut parser = Parser::new(tokens, &path);
    parser.parse().unwrap()
}

#[test]
fn test_parse_proc_with_params() {
    let file = parse_source("[proc,my_proc](int $a, string $b)(int)\nreturn($a);");
    assert_eq!(file.scripts.len(), 1);
    assert_eq!(file.scripts[0].trigger, "proc");
    assert_eq!(file.scripts[0].name, "my_proc");
    assert_eq!(file.scripts[0].params.len(), 2);
    assert_eq!(file.scripts[0].return_types.len(), 1);
}

#[test]
fn test_parse_if_else() {
    let source = r#"
[proc,test_else](int $x)(int)
if ($x > 0) {
    return(1);
} else {
    return(0);
}
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    // Check that the if statement has an else body
    let stmt = &file.scripts[0].body[0];
    match stmt {
        runec::parser::Statement::If { else_body, .. } => {
            assert!(else_body.is_some(), "Expected else body");
        }
        _ => panic!("Expected If statement"),
    }
}

#[test]
fn test_parse_while_loop() {
    let source = r#"
[proc,loop_test](int $n)(int)
def_int $i = 0;
while ($i < $n) {
    $i = calc($i + 1);
}
return($i);
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    assert_eq!(file.scripts[0].body.len(), 3); // def, while, return
}

#[test]
fn test_parse_switch() {
    let source = r#"
[proc,switch_test](int $x)(int)
switch_int ($x) {
    case 1:
        return(10);
    case 2:
        return(20);
    default:
        return(0);
}
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    let stmt = &file.scripts[0].body[0];
    match stmt {
        runec::parser::Statement::Switch { cases, default, .. } => {
            assert_eq!(cases.len(), 2);
            assert!(default.is_some());
        }
        _ => panic!("Expected Switch statement"),
    }
}

#[test]
fn test_parse_multiple_triggers() {
    let source = r#"
[proc,helper](int $n)(int)
return($n);

[label,my_label]
~helper(5);

[clientscript,my_cs]
~helper(10);
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 3);
    assert_eq!(file.scripts[0].trigger, "proc");
    assert_eq!(file.scripts[1].trigger, "label");
    assert_eq!(file.scripts[2].trigger, "clientscript");
}

#[test]
fn test_parse_boolean_literals() {
    let source = r#"
[proc,bool_test](int $x)(boolean)
if ($x > 0) {
    return(true);
}
return(false);
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
}

#[test]
fn test_parse_null_literal() {
    let source = r#"
[proc,null_test](int $x)(int)
if ($x = null) {
    return(0);
}
return($x);
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
}

#[test]
fn test_parse_game_var() {
    let source = r#"
[proc,var_test]
%my_var = 42;
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
}

#[test]
fn test_parse_constant_var() {
    let source = r#"
[proc,const_test](int $x)(int)
return(calc($x + ^max_value));
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
}

#[test]
fn test_parse_jump_call() {
    let source = r#"
[proc,jump_test]
@some_label(5);
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
}

#[test]
fn test_parse_gosub_call() {
    let source = r#"
[proc,gosub_test](int $n)(int)
return(~other_proc($n));
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
}

#[test]
fn test_parse_else_if_chain() {
    let source = r#"
[proc,elseif_test](int $x)(int)
if ($x = 1) {
    return(10);
} else if ($x = 2) {
    return(20);
} else if ($x = 3) {
    return(30);
} else {
    return(0);
}
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    let stmt = &file.scripts[0].body[0];
    match stmt {
        runec::parser::Statement::If {
            else_if, else_body, ..
        } => {
            assert_eq!(else_if.len(), 2, "Expected 2 else-if branches");
            assert!(else_body.is_some(), "Expected else body");
        }
        _ => panic!("Expected If statement"),
    }
}

#[test]
fn test_parse_var_declaration_types() {
    let source = r#"
[proc,types_test]
def_int $a = 1;
def_string $b = "hello";
def_boolean $c = true;
def_loc $d;
def_npc $e;
def_obj $f;
def_coord $g;
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    assert_eq!(file.scripts[0].body.len(), 7);
}

#[test]
fn test_parse_no_return_type() {
    let source = r#"
[proc,void_proc]
def_int $x = 5;
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    assert!(file.scripts[0].return_types.is_empty());
}

#[test]
fn test_parse_multi_return() {
    let source = r#"
[proc,multi_ret](int $x)(int, string)
return($x, "hello");
"#;
    let file = parse_source(source);
    assert_eq!(file.scripts.len(), 1);
    assert_eq!(file.scripts[0].return_types.len(), 2);
}
