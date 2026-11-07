use runec::ide::{SemanticToken, TokenKind};
use runec::lsp::project_from_json;
use runec::project::Project;
use runec::sources::Scope;
use serde_json::json;

fn project() -> Project {
    project_from_json(&json!({
        "commands":[
            {"name":"mes","opcode":1,"params":["string"]},
            {"name":"queue","opcode":2,"params":["queue","int"]},
            {"name":"npc_uid","opcode":3,"returns":["npc_uid"]},
            {"name":"touch","opcode":4,"params":["component"]},
            {"name":"item","opcode":5,"params":["obj"]},
            {"name":"displayname","opcode":6,"returns":["string"]}
        ],
        "entities":[
            {"name":"bank:com1","type":"component","id":1,"scope":"global"},
            {"name":"cheese+tom_batta","type":"obj","id":2,"scope":"global"}
        ]
    }))
    .unwrap()
}

fn at<'a>(text: &str, tokens: &'a [SemanticToken], needle: &str) -> &'a SemanticToken {
    let start = text.find(needle).unwrap();
    tokens
        .iter()
        .find(|token| token.range == (start..start + needle.len()))
        .unwrap_or_else(|| panic!("No complete token for {needle:?}: {tokens:?}"))
}

fn ordered(text: &str, tokens: &[SemanticToken]) {
    let mut end = 0;
    for token in tokens {
        assert!(token.range.start >= end, "overlapping token: {token:?}");
        assert!(!token.range.is_empty());
        assert!(text.get(token.range.clone()).is_some());
        end = token.range.end;
    }
}

#[test]
fn scripts_keep_reference_categories_and_complete_sigils() {
    let mut project = project();
    let text = "[proc,test](int $arg)(int)\ndef_int $local = ^missing;\n%bank_tab = $arg;\nmes(\"hi\");\n~unknown;\n@destination;\ntouch(bank:com1);\nitem(cheese+tom_batta);\nreturn($local);\n";
    let doc = project.open("mem:test", Scope::Global, 1, text);
    let tokens = project.semantic_tokens(doc);
    ordered(text, &tokens);
    assert_eq!(at(text, &tokens, "[").kind, TokenKind::Decorator);
    assert_eq!(at(text, &tokens, "proc").kind, TokenKind::Type);
    assert!(at(text, &tokens, "test").declaration);
    assert_eq!(at(text, &tokens, "$arg").kind, TokenKind::Parameter);
    assert_eq!(at(text, &tokens, "$local").kind, TokenKind::Variable);
    assert!(at(text, &tokens, "^missing").static_);
    assert!(at(text, &tokens, "^missing").readonly);
    assert!(at(text, &tokens, "%bank_tab").static_);
    assert!(!at(text, &tokens, "%bank_tab").readonly);
    assert!(at(text, &tokens, "mes").default_library);
    for name in ["~unknown", "@destination"] {
        assert_eq!(at(text, &tokens, name).kind, TokenKind::Function);
        assert!(!at(text, &tokens, name).default_library);
    }
    for name in ["bank:com1", "cheese+tom_batta"] {
        assert_eq!(at(text, &tokens, name).kind, TokenKind::EnumMember);
    }
}

#[test]
fn overloaded_types_and_trigger_subjects_are_contextual() {
    let mut project = project();
    let text = "[proc,test](queue $q)(npc_uid)\nqueue(missing_queue, 1);\nreturn(npc_uid);\n[opnpc1,goblin]\nreturn;\n";
    let doc = project.open("mem:test", Scope::Global, 1, text);
    let tokens = project.semantic_tokens(doc);
    ordered(text, &tokens);
    assert_eq!(at(text, &tokens, "queue").kind, TokenKind::Type);
    assert_eq!(at(text, &tokens, "npc_uid").kind, TokenKind::Type);
    for name in ["queue", "npc_uid"] {
        let start = if name == "queue" {
            text.find("\nqueue").unwrap() + 1
        } else {
            text.rfind(name).unwrap()
        };
        let token = tokens
            .iter()
            .find(|token| token.range.start == start)
            .unwrap();
        assert_eq!(token.kind, TokenKind::Function);
        assert!(token.default_library);
    }
    assert_eq!(at(text, &tokens, "missing_queue").kind, TokenKind::Function);
    assert!(!at(text, &tokens, "missing_queue").default_library);
    assert_eq!(at(text, &tokens, "goblin").kind, TokenKind::EnumMember);
}

#[test]
fn interpolated_strings_have_tags_nested_code_and_no_overlaps() {
    let mut project = project();
    let text = r#"[proc,test](int $value)
mes("😀 <col=ff0000>Hi</col><br><p,neutral> <$value> <displayname> <calc(2 > 1)> <mes("inner <displayname>")> \<literal>");
mes(r"<displayname>");
"#;
    let doc = project.open("mem:test", Scope::Global, 1, text);
    let tokens = project.semantic_tokens(doc);
    ordered(text, &tokens);
    for tag in ["<col=ff0000>", "</col>", "<br>", "<p,neutral>"] {
        assert_eq!(at(text, &tokens, tag).kind, TokenKind::Keyword);
    }
    assert!(at(text, &tokens, "displayname").default_library);
    assert_eq!(at(text, &tokens, "calc").kind, TokenKind::Keyword);
    assert_eq!(
        at(text, &tokens, "r\"<displayname>\"").kind,
        TokenKind::String
    );
    let escaped = text.find("\\<literal>").unwrap();
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == TokenKind::String && token.range.contains(&escaped))
    );
    let nested = text.find("inner <displayname>").unwrap() + "inner <".len();
    let token = tokens
        .iter()
        .find(|token| token.range.start == nested)
        .unwrap();
    assert!(token.default_library);
    assert!(project.hover(doc, nested).is_some());
}

#[test]
fn incomplete_literals_comments_and_directives_remain_highlighted() {
    let mut project = project();
    for (suffix, kind) in [
        ("/* unfinished\n comment 😀", TokenKind::Comment),
        ("\"unfinished 😀", TokenKind::String),
        ("r\"unfinished <mes>", TokenKind::String),
        ("'a", TokenKind::String),
        ("#testscript", TokenKind::Macro),
    ] {
        let text = format!("[proc,test]\n{suffix}");
        let doc = project.open("mem:test", Scope::Global, 1, text.clone());
        let tokens = project.semantic_tokens(doc);
        ordered(&text, &tokens);
        assert_eq!(at(&text, &tokens, suffix).kind, kind);
        project.close(doc);
    }
}

#[test]
fn config_values_that_name_types_are_types() {
    let mut project = project();
    let text = "[varc_1]\ntype=int\nscope=perm\n";
    let doc = project.open_config("mem:all.varc", Scope::Global, 1, text, "varc");
    let tokens = project.semantic_tokens(doc);
    assert_eq!(at(text, &tokens, "int").kind, TokenKind::Type);
    assert_eq!(at(text, &tokens, "perm").kind, TokenKind::EnumMember);

    let text = "[enum_1]\ninputtype=namedobj\noutputtype=graphic\n";
    let doc = project.open_config("mem:all.enum", Scope::Global, 1, text, "enum");
    let tokens = project.semantic_tokens(doc);
    for word in ["namedobj", "graphic"] {
        assert_eq!(at(text, &tokens, word).kind, TokenKind::Type, "{word}");
    }

    let text = "[table]\ncolumn=string,coord,LIST\n";
    let doc = project.open_config("mem:all.dbtable", Scope::Global, 1, text, "dbtable");
    let tokens = project.semantic_tokens(doc);
    assert_eq!(at(text, &tokens, "string").kind, TokenKind::EnumMember);
    assert_eq!(at(text, &tokens, "coord").kind, TokenKind::Type);
    assert_eq!(at(text, &tokens, "LIST").kind, TokenKind::EnumMember);
}

#[test]
fn interface_values_read_by_their_keys() {
    let mut project = project();
    let text = "// 1:0\n[com_0]\nlayer=com_1\ngraphic=\"v2_stone,1\"\ntext=a // b, c\n\
                onload=graphic_swapper(event_com, \"x<br>y\", ^white, 5){varplayer_2}\n";
    let doc = project.open_config("mem:bank.if3", Scope::Global, 1, text, "if3");
    let tokens = project.semantic_tokens(doc);
    ordered(text, &tokens);
    assert_eq!(at(text, &tokens, "// 1:0").kind, TokenKind::Comment);
    assert_eq!(at(text, &tokens, "com_0").kind, TokenKind::Namespace);
    assert_eq!(at(text, &tokens, "com_1").kind, TokenKind::EnumMember);
    assert_eq!(at(text, &tokens, "v2_stone,1").kind, TokenKind::EnumMember);
    assert_eq!(at(text, &tokens, "a // b, c").kind, TokenKind::String);
    assert_eq!(
        at(text, &tokens, "graphic_swapper").kind,
        TokenKind::Function
    );
    assert_eq!(at(text, &tokens, "event_com").kind, TokenKind::EnumMember);
    assert_eq!(at(text, &tokens, "\"x<br>y\"").kind, TokenKind::String);
    assert_eq!(at(text, &tokens, "^white").kind, TokenKind::Variable);
    assert_eq!(at(text, &tokens, "5").kind, TokenKind::Number);
    assert_eq!(at(text, &tokens, "varplayer_2").kind, TokenKind::EnumMember);
}

#[test]
fn world_map_values_read_by_their_keys() {
    let mut project = project();
    let flu = project.open_config("mem:all.flu", Scope::Global, 1, "[underlay_47]\n", "flu");
    let flo = project.open_config("mem:all.flo", Scope::Global, 1, "[overlay_9]\n", "flo");
    let text = "[file_1]\nformat=area\nunderlays=1\nflu=underlay_47\noverlays=1\nflo=overlay_9\n\
                square=main/file_1/m50_50.fms\n";
    let area = project.open_config("mem:main.fm", Scope::Global, 1, text, "fm");
    let tokens = project.semantic_tokens(area);
    ordered(text, &tokens);
    assert_eq!(at(text, &tokens, "file_1").kind, TokenKind::Namespace);
    assert_eq!(
        at(text, &tokens, "main/file_1/m50_50.fms").kind,
        TokenKind::String
    );
    // The section is a file id, not a name anything refers to.
    assert!(project.hover(area, text.find("file_1").unwrap()).is_none());
    let underlay = text.find("underlay_47").unwrap();
    assert_eq!(project.definition(area, underlay).unwrap().source, flu);

    let text = "tile3200_3215=underlay:underlay_47\ntile3200_3216=layers:2,overlay,locs,extra:3\n\
                overlay=overlay_9\nshape_rotation=1,1\ntile3200_3217=overlay:direct\nfloor=overlay_9\n\
                tile3200_3218=overlay:overlay_9@5\n";
    let square = project.open_config("mem:m50_50.fms", Scope::Global, 1, text, "fms");
    let tokens = project.semantic_tokens(square);
    ordered(text, &tokens);
    let at_part = |within: &str, part: &str| {
        let start = text.find(within).unwrap() + within.find(part).unwrap();
        start..start + part.len()
    };
    let kind = |within: &str, part: &str| {
        let range = at_part(within, part);
        tokens
            .iter()
            .find(|token| token.range == range)
            .unwrap_or_else(|| panic!("No token for {part:?} in {within:?}: {tokens:?}"))
            .kind
    };
    assert_eq!(kind("=underlay:", "underlay"), TokenKind::Keyword);
    assert_eq!(kind(":underlay_47", "underlay_47"), TokenKind::EnumMember);
    assert_eq!(kind("=layers:2", "layers"), TokenKind::Keyword);
    assert_eq!(kind("layers:2", "2"), TokenKind::Number);
    assert_eq!(kind(",locs", "locs"), TokenKind::Keyword);
    assert_eq!(kind("extra:3", "3"), TokenKind::Number);
    assert_eq!(kind(":direct", "direct"), TokenKind::Keyword);
    assert_eq!(kind("@5", "5"), TokenKind::Number);
    for (within, part, config) in [
        (":underlay_47", "underlay_47", flu),
        ("floor=overlay_9", "overlay_9", flo),
        (":overlay_9@", "overlay_9", flo),
    ] {
        let start = at_part(within, part).start;
        assert_eq!(project.definition(square, start).unwrap().source, config);
    }
    assert!(
        project
            .definition(square, at_part("@5", "5").start)
            .is_none()
    );
}

#[test]
fn config_text_is_one_string_and_stops_at_a_comment() {
    let mut project = project();
    let text = "[man]\nop1=Talk-to the man, slowly // spoken\nname=Man\nsize=2\nmembers=yes\n";
    let doc = project.open_config("mem:all.npc", Scope::Global, 1, text, "npc");
    let tokens = project.semantic_tokens(doc);
    ordered(text, &tokens);
    assert_eq!(
        at(text, &tokens, "Talk-to the man, slowly").kind,
        TokenKind::String
    );
    assert_eq!(at(text, &tokens, "// spoken").kind, TokenKind::Comment);
    assert_eq!(at(text, &tokens, "Man").kind, TokenKind::String);
    assert_eq!(at(text, &tokens, "2").kind, TokenKind::Number);
    assert_eq!(at(text, &tokens, "yes").kind, TokenKind::Keyword);
}

#[test]
fn configs_and_packs_preserve_sections_keys_constants_and_comments() {
    let mut project = project();
    let text = "/* hidden\n[not_a_definition]\n*/\n[cheese+tom_batta]\nname=\"Food 😀\"\ncost=-10\ncoord=0_50_50_10_10\nactive=yes\nparam=^unknown,%var\nmodel=food_model // comment\n";
    let doc = project.open_config("mem:items.obj", Scope::Global, 1, text, "obj");
    let tokens = project.semantic_tokens(doc);
    ordered(text, &tokens);
    assert_eq!(
        at(text, &tokens, "/* hidden\n[not_a_definition]\n*/").kind,
        TokenKind::Comment
    );
    assert_eq!(
        at(text, &tokens, "cheese+tom_batta").kind,
        TokenKind::Namespace
    );
    assert!(at(text, &tokens, "cheese+tom_batta").declaration);
    assert_eq!(at(text, &tokens, "cost").kind, TokenKind::Property);
    assert_eq!(at(text, &tokens, "-10").kind, TokenKind::Number);
    assert_eq!(at(text, &tokens, "0_50_50_10_10").kind, TokenKind::Number);
    assert_eq!(at(text, &tokens, "yes").kind, TokenKind::Keyword);
    assert!(at(text, &tokens, "^unknown").readonly);
    assert!(at(text, &tokens, "%var").static_);
    assert_eq!(at(text, &tokens, "food_model").kind, TokenKind::EnumMember);
    assert!(project.workspace_symbols("not_a_definition", 10).is_empty());
    let pack = "123:4=bank:com4\n";
    let doc = project.open_config(
        "mem:interface.pack",
        Scope::Global,
        1,
        pack,
        "pack:interface",
    );
    let tokens = project.semantic_tokens(doc);
    ordered(pack, &tokens);
    assert_eq!(at(pack, &tokens, "123:4").kind, TokenKind::Property);
    assert_eq!(at(pack, &tokens, "bank:com4").kind, TokenKind::EnumMember);
}

#[test]
fn partial_edits_keep_tokens_ordered_on_utf8_boundaries() {
    let mut project = project();
    for config in [false, true] {
        let text = if config {
            "[entry]\nname=\"escaped \\😀\"\n/* multi\nline */\nparam=^value,%other\n"
        } else {
            "[proc,entry](int $n)\nmes(\"😀 <mes(\"nested <$n>\")>\");\n/* multi\nline */\n#testscript\n"
        };
        let doc = if config {
            project.open_config("mem:editing.obj", Scope::Global, 1, "", "obj")
        } else {
            project.open("mem:editing", Scope::Global, 1, "")
        };
        for end in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
            project.update(doc, end as i64 + 2, &text[..end]).unwrap();
            ordered(&text[..end], &project.semantic_tokens(doc));
        }
    }
}
