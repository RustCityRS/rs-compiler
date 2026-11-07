//! Filesystem-independent embedding example.
fn main() {
    use runec::environment::{EngineProfile, Trigger};
    use runec::project::{Options, Project, Target};
    use runec::sources::{Scope, Visibility};
    use runec::symbol::SymbolRegistry;
    use runec::types::Type;

    let mut registry = SymbolRegistry::new();
    let mut engine = EngineProfile::default();
    engine.triggers.insert(
        "proc".into(),
        Trigger {
            byte: Some(0), // Choose the trigger encoding used by your emulator.
            allows_returns: true,
            ..Default::default()
        },
    );
    registry.set_engine_profile(engine);
    registry.set_script_id("proc", "item", 7);
    registry.register_entity_id("bronze_sword".into(), Type::Obj, 1205);
    let mut visibility = Visibility::new();
    visibility.declare("obj", "bronze_sword", Scope::Global);
    registry.set_visibility(visibility);

    let mut project = Project::new(Target::Rune, registry, Options::default());
    let source = project.open(
        "database:item",
        Scope::Global,
        1,
        "[proc,item]()(obj) return(bronze_sword);",
    );
    let result = project.compile();
    for diagnostic in result.analysis.diagnostics.diagnostics() {
        eprintln!("{}", project.render_diagnostic(diagnostic));
    }
    assert!(!result.analysis.cancelled && !result.analysis.diagnostics.has_errors());
    assert_eq!(result.artifacts[0].id, 7);
    // Store result.artifacts yourself. Each contains source, ID, name and bytes.
    // Persist result.analysis.assignments if you want the compiler's new IDs.
    project
        .update(source, 2, "[proc,item]()(obj) return(null);")
        .unwrap();
}
