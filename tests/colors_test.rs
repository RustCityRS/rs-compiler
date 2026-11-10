use runec::ide::Color;
use runec::project::{Options, Project, SourceId, Target};
use runec::sources::Scope;
use runec::symbol::SymbolRegistry;
use runec::types::Type;

fn project() -> Project {
    let mut registry = SymbolRegistry::new();
    registry.register_command("paint".into(), 1, vec![Type::Colour], vec![]);
    registry.register_command("ignore".into(), 2, vec![Type::Int], vec![]);
    Project::new(Target::Rune, registry, Options::default())
}

fn previews(project: &Project, source: SourceId) -> Vec<(String, Color)> {
    let document = project.document(source).unwrap();
    project
        .document_colors(source)
        .into_iter()
        .map(|color| (document.text[color.range].to_string(), color.color))
        .collect()
}

const RED: Color = Color {
    red: 1.0,
    green: 0.0,
    blue: 0.0,
    alpha: 1.0,
};
const GREEN: Color = Color {
    red: 0.0,
    green: 1.0,
    blue: 0.0,
    alpha: 1.0,
};

#[test]
fn script_colors_follow_types_constants_and_unsaved_definitions() {
    let mut project = project();
    let constants = project.open_config(
        "colour.constant",
        Scope::Global,
        1,
        "^red_rgb=0xFF0000\n^green_rgb=0xFF00\n",
        "constant",
    );
    let text = "[proc,test]()(rgb)\ndef_rgb $value = ^red_rgb;\npaint(/* colour */ 16711680);\nignore(0xff0000);\nignore(^red_rgb);\n// paint(0xff0000);\nreturn(0x00ff00);\n";
    let source = project.open("test.rs2", Scope::Rune, 1, text);
    assert_eq!(
        previews(&project, constants),
        vec![("0xFF0000".into(), RED), ("0xFF00".into(), GREEN)]
    );
    assert_eq!(
        previews(&project, source),
        vec![
            ("^red_rgb".into(), RED),
            ("16711680".into(), RED),
            ("^red_rgb".into(), RED),
            ("0x00ff00".into(), GREEN)
        ]
    );
    let colors = project.document_colors(source);
    assert!(
        project
            .color_presentation(source, colors[0].range.clone(), GREEN)
            .is_none()
    );
    project
        .update(constants, 2, "^red_rgb=0x00FF00\n^green_rgb=0xFF00\n")
        .unwrap();
    assert_eq!(previews(&project, source)[0].1, GREEN);
}

#[test]
fn cache_color_formats_preserve_encoding_and_ignore_palette_ids() {
    let mut project = project();
    let model = project.open_config("test.obj", Scope::Rune, 1, "[test]\nrecol1s=31744\nrecolhsl1d=960\nrecolhsl2d=65536\nrecolpalette1=10\nretex1s=31744\n", "obj");
    let colors = project.document_colors(model);
    assert_eq!(colors.len(), 2);
    assert_eq!(colors[0].color, RED);
    assert!(
        colors[1].color.red > 0.97 && colors[1].color.green < 0.2 && colors[1].color.blue < 0.1
    );
    assert_eq!(
        project
            .color_presentation(model, colors[0].range.clone(), GREEN)
            .unwrap()
            .replacement,
        "992"
    );
    assert_eq!(
        project
            .color_presentation(model, colors[1].range.clone(), GREEN)
            .unwrap()
            .replacement,
        "22464"
    );
    assert_eq!(
        project
            .color_presentation(model, colors[1].range.clone(), colors[1].color)
            .unwrap()
            .replacement,
        "960"
    );
    let interface = project.open_config(
        "test.if3",
        Scope::Rune,
        1,
        "[text]\ncolour=0xffffffff\n",
        "if3",
    );
    let color = project.document_colors(interface).remove(0);
    assert_eq!(color.color.alpha, 1.0);
    assert_eq!(
        project
            .color_presentation(interface, color.range, GREEN)
            .unwrap()
            .replacement,
        "0xff00ff00"
    );
    let particle = project.open_config(
        "test.particleemitter",
        Scope::Rune,
        1,
        "[test]\nrgbatarget=-65536\n",
        "particleemitter",
    );
    let color = project.document_colors(particle).remove(0);
    assert_eq!(color.color, RED);
    assert_eq!(
        project
            .color_presentation(
                particle,
                color.range,
                Color {
                    alpha: 128.0 / 255.0,
                    ..GREEN
                }
            )
            .unwrap()
            .replacement,
        "-2147418368"
    );
    project
        .update(particle, 2, "[test]\nrgbatarget=16711680\n")
        .unwrap();
    let color = project.document_colors(particle).remove(0);
    assert_eq!(
        project
            .color_presentation(particle, color.range, GREEN)
            .unwrap()
            .replacement,
        "-16711936"
    );
    let npc = project.open_config(
        "test.npc",
        Scope::Rune,
        1,
        "[test]\nspotshadowcolour=960,960\n",
        "npc",
    );
    let color = project.document_colors(npc).remove(0);
    assert_eq!(
        project
            .color_presentation(
                npc,
                color.range,
                Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 1.0,
                    alpha: 1.0
                }
            )
            .unwrap()
            .replacement,
        "-21568"
    );
}

#[test]
fn declared_config_colors_and_conditional_texture_fields() {
    let mut project = project();
    let source = project.open_config(
        "test.enum",
        Scope::Global,
        1,
        "[test]\ninputtype=int\noutputtype=rgb\nval=16711680,16711680\ndefault=65280\n",
        "enum",
    );
    assert_eq!(
        previews(&project, source),
        vec![("16711680".into(), RED), ("65280".into(), GREEN)]
    );
    let source = project.open_config("test.texture", Scope::Global, 1, "[texture]\ncolor=1\n[op1]\ntype=constant_color\ncolor=0xff0000\n[op2]\ntype=color_replace\ntarget_color=0x00ff00\n", "texture");
    assert_eq!(
        previews(&project, source),
        vec![("0xff0000".into(), RED), ("0x00ff00".into(), GREEN)]
    );
    let source = project.open_config(
        "test.wma",
        Scope::Global,
        1,
        "[0]\nbackground=0xffffffff\n[1]\nbackground=0xff0000\n",
        "wma",
    );
    assert_eq!(previews(&project, source), vec![("0xff0000".into(), RED)]);
    let source = project.open_config(
        "test.if",
        Scope::Rune,
        1,
        "[test]\ncolour=ff0000\nactivecolour=00ff00\n",
        "if",
    );
    assert_eq!(
        previews(&project, source),
        vec![("ff0000".into(), RED), ("00ff00".into(), GREEN)]
    );
    let source = project.open(
        "tags.rs2",
        Scope::Rune,
        1,
        "[proc,tags]\nmes(\"@red@red@gre@green\");\n",
    );
    assert_eq!(
        previews(&project, source),
        vec![("@red@".into(), RED), ("@gre@".into(), GREEN)]
    );
}

#[test]
fn map_model_loading_and_procedural_texture_colors() {
    let mut project = project();
    let source = project.open_config("test.jm2", Scope::Rune, 1, "==== ENV ====\nsun_color=0xff0000\nfog_color=0x00ff00\n==== LIGHTS ====\n0: hsl=960 colors=960,960\n", "jm2");
    let colors = previews(&project, source);
    assert_eq!(colors.len(), 3);
    assert_eq!(colors[0].1, RED);
    assert_eq!(colors[1].1, GREEN);
    let source = project.open_config(
        "test.model",
        Scope::Rune,
        1,
        "face0.colour=960\nface0.texture=960\n",
        "model",
    );
    assert_eq!(project.document_colors(source).len(), 1);
    let source = project.open_config(
        "test.screen",
        Scope::Rune,
        1,
        "[text]\ntext_colour=4294901760\nwidth=4294901760\n",
        "screen",
    );
    assert_eq!(previews(&project, source), vec![("4294901760".into(), RED)]);
    let source = project.open_config("test.texture", Scope::Rune, 1, "[op1]\ntype=gradient_map\nstops=custom;0,0xff0000;4096,0x00ff00\n[op2]\ntype=vector_graphics\nshapes=rectangle,0,0,4096,4096,2048,-1,1\n[op3]\ntype=vector_graphics\nshapes=line,0,0,4096,4096,0xff0000,1\nmonochrome=no\n", "texture");
    let colors = previews(&project, source);
    assert_eq!(colors.len(), 4);
    assert_eq!(colors[0].1, RED);
    assert_eq!(colors[1].1, GREEN);
    assert_eq!(
        colors[2].1,
        Color {
            red: 0.5,
            green: 0.5,
            blue: 0.5,
            alpha: 1.0
        }
    );
    assert_eq!(colors[3].1, RED);
    let source = project.open_config(
        "floorcol.txt",
        Scope::Rune,
        1,
        "cliff=00000038,00ff0000\n",
        "floorcol",
    );
    let colors = project.document_colors(source);
    assert_eq!(colors.len(), 2);
    assert_eq!(
        colors[0].color,
        Color {
            red: 0.65625,
            green: 0.65625,
            blue: 0.65625,
            alpha: 1.0
        }
    );
    assert_eq!(colors[1].color, RED);
    assert_eq!(
        project
            .color_presentation(source, colors[0].range.clone(), colors[0].color)
            .unwrap()
            .replacement,
        "00000038"
    );
    assert_eq!(
        project
            .color_presentation(source, colors[1].range.clone(), GREEN)
            .unwrap()
            .replacement,
        "0000ff00"
    );
}
