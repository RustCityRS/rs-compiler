//! Source discovery and visibility for the three script namespaces.
use crate::parser::Expr;
use crate::symbol::{SymbolKind, SymbolRegistry};
use crate::symloader::{Comments, collect_all_files};
use crate::types::Type;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::BufRead;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Rune = 1,
    Client = 2,
    Global = 4,
}

impl Scope {
    pub fn folder(self) -> &'static str {
        match self {
            Self::Rune => "runescript",
            Self::Client => "clientscript",
            Self::Global => "globalscript",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Rune => "rs2",
            Self::Client => "cs2",
            Self::Global => "gs2",
        }
    }

    fn at(path: &Path) -> Option<Self> {
        match path.file_name()?.to_str()? {
            "runescript" => Some(Self::Rune),
            "clientscript" => Some(Self::Client),
            "globalscript" => Some(Self::Global),
            _ => None,
        }
    }
}

fn namespace(path: &Path) -> Option<(&Path, Scope)> {
    let candidates = || {
        path.ancestors()
            .filter_map(|path| Scope::at(path).map(|scope| (path, scope)))
    };
    candidates()
        .find(|(path, _)| {
            path.parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "scripts")
        })
        .or_else(|| candidates().next())
}

/// Accept the scripts parent as well as a target's own source directory.
pub(crate) fn target_dir(dir: &Path, scope: Scope) -> PathBuf {
    let target = dir.join(scope.folder());
    if Scope::at(dir).is_none() && target.is_dir() {
        target
    } else {
        dir.to_path_buf()
    }
}

/// The target's sources plus its sibling globalscript directory, when present.
pub fn roots(dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![dir.to_path_buf()];
    if let Some((root, scope)) = namespace(dir)
        && scope != Scope::Global
        && let Some(parent) = root.parent()
    {
        let global = parent.join(Scope::Global.folder());
        if global.is_dir() {
            roots.insert(0, global);
        }
    }
    roots
}

pub fn config_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots(dir) {
        collect_all_files(&root, &mut files);
    }
    files
}

pub fn script_files(dir: &Path, extension: &str) -> io::Result<Vec<PathBuf>> {
    let dir = target_dir(
        dir,
        if extension == "cs2" {
            Scope::Client
        } else {
            Scope::Rune
        },
    );
    let mut files = Vec::new();
    for root in roots(&dir) {
        // Report a missing source root instead of silently compiling nothing.
        fs::read_dir(&root)?;
        let mut candidates = Vec::new();
        collect_all_files(&root, &mut candidates);
        for path in candidates {
            let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !matches!(ext, "rs2" | "cs2" | "gs2") {
                continue;
            }
            if let Some((_, scope)) = namespace(&path)
                && ext != scope.extension()
            {
                return Err(io::Error::other(format!(
                    "{}: .{ext} source belongs in scripts/{}, not scripts/{}",
                    path.display(),
                    match ext {
                        "rs2" => "runescript",
                        "cs2" => "clientscript",
                        _ => "globalscript",
                    },
                    scope.folder(),
                )));
            }
            if ext == extension || ext == "gs2" {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Pack registries assign IDs; source declarations grant visibility. Keeping
/// this separate prevents stale or opposite-target pack entries granting access.
#[derive(Debug, Clone, Default)]
pub struct Visibility {
    active: bool,
    documents: HashMap<PathBuf, Scope>,
    owners: HashMap<String, HashMap<String, u8>>,
    ids: HashMap<(Type, i32), String>,
}

fn config_kind(ty: Type) -> Option<&'static str> {
    match ty {
        Type::NamedObj | Type::Obj => Some("obj"),
        Type::IdKit => Some("idk"),
        Type::MapElement => Some("mel"),
        Type::DbColumn => Some("dbtable"),
        Type::Npc
        | Type::Loc
        | Type::Seq
        | Type::Spotanim
        | Type::MesAnim
        | Type::Inv
        | Type::Enum
        | Type::Struct
        | Type::Param
        | Type::DbTable
        | Type::DbRow
        | Type::Varp
        | Type::Varn
        | Type::Vars
        | Type::Varbit
        | Type::Hunt
        | Type::Controller
        | Type::Hitmark => Some(ty.name()),
        _ => None,
    }
}

impl Visibility {
    /// Explicit ownership for memory documents and caller-defined namespaces.
    pub fn new() -> Self {
        Self {
            active: true,
            ..Self::default()
        }
    }

    /// Labels are opaque identifiers. No path lookup or filesystem access occurs.
    pub fn bind(&mut self, label: impl Into<PathBuf>, scope: Scope) {
        self.active = true;
        self.documents.insert(label.into(), scope);
    }

    pub fn unbind(&mut self, label: &Path) {
        self.documents.remove(label);
    }

    fn scope(&self, label: &Path) -> Option<Scope> {
        self.documents
            .get(label)
            .copied()
            .or_else(|| namespace(label).map(|(_, scope)| scope))
    }

    pub(crate) fn undeclare(&mut self, kind: &str, name: &str, scope: Scope) {
        if let Some(names) = self.owners.get_mut(kind)
            && let Some(owners) = names.get_mut(name)
        {
            *owners &= !(scope as u8);
            if *owners == 0 {
                names.remove(name);
            }
        }
    }

    pub(crate) fn load(dir: &Path, registry: &SymbolRegistry) -> Self {
        let Some((root, _)) = namespace(dir) else {
            return Self::default();
        };
        let Some(parent) = root.parent() else {
            return Self::default();
        };
        let mut result = Self {
            active: true,
            ..Self::default()
        };
        for scope in [Scope::Global, Scope::Rune, Scope::Client] {
            let mut files = Vec::new();
            collect_all_files(&parent.join(scope.folder()), &mut files);
            for file in files {
                let Some(ext) = file.extension().and_then(|ext| ext.to_str()) else {
                    continue;
                };
                let is_script = matches!(ext, "rs2" | "cs2" | "gs2");
                if !is_script
                    && ext != "constant"
                    && ext != "varc_int"
                    && ext != "varc_string"
                    && !ext.starts_with("clansetting_")
                    && !Type::from_name(ext).is_some_and(|ty| config_kind(ty).is_some())
                    && ext != "mel"
                {
                    continue;
                }
                let Ok(input) = fs::File::open(&file) else {
                    continue;
                };
                let mut input = io::BufReader::new(input);
                let mut buffer = String::new();
                let mut comments = Comments::default();
                loop {
                    buffer.clear();
                    if !input.read_line(&mut buffer).is_ok_and(|count| count > 0) {
                        break;
                    }
                    let Some(line) = comments.line(&buffer) else {
                        continue;
                    };
                    if is_script {
                        if let Some(header) =
                            line.strip_prefix('[').and_then(|line| line.split_once(']'))
                            && let Some((trigger, name)) = header.0.split_once(',')
                            && trigger.trim() != "command"
                        {
                            result.declare(trigger.trim(), name.trim(), scope);
                        }
                    } else if ext == "constant" {
                        if let Some((name, _)) = line.split_once('=') {
                            result.declare("constant", name.trim().trim_start_matches('^'), scope);
                        }
                    } else if let Some(name) = line
                        .strip_prefix('[')
                        .and_then(|line| line.strip_suffix(']'))
                    {
                        result.declare(ext, name, scope);
                    }
                }
            }
        }
        result.index_registry(registry);
        result
    }

    /// Index typed numeric references after the caller has supplied config IDs.
    pub fn index_registry(&mut self, registry: &SymbolRegistry) {
        self.ids.clear();
        for (name, entry) in &registry.entity_ids {
            for &(ty, id) in &entry.variants {
                if config_kind(ty).is_some() {
                    self.ids.insert((ty, id), name.clone());
                }
                // Certificate configs are generated by the object packer from
                // the base object. They inherit its namespace when there is no
                // explicit certificate definition.
                if ty == Type::Obj
                    && let Some(base) = name.strip_prefix("cert_")
                    && let Some(objects) = self.owners.get_mut("obj")
                    && let Some(&owner) = objects.get(base)
                {
                    objects.entry(name.clone()).or_insert(owner);
                }
            }
        }
        for (name, symbol) in &registry.game_vars {
            if let SymbolKind::GameVar {
                var_id, category, ..
            } = &symbol.kind
                && let Some(ty) = Type::from_name(category)
            {
                self.ids.insert((ty, *var_id), name.clone());
            }
        }
    }

    pub(crate) fn declare_source(&mut self, path: &Path, file: &crate::parser::ScriptFile) {
        for script in &file.scripts {
            self.declare_script(path, &script.trigger, &script.name);
        }
    }

    pub(crate) fn declare_script(&mut self, path: &Path, trigger: &str, name: &str) {
        if let Some(scope) = self.scope(path) {
            self.declare(trigger, name, scope);
        }
    }

    pub(crate) fn declaration(&self, path: &Path, trigger: &str, name: &str) -> Option<String> {
        self.check(path, trigger, name, false)
    }

    pub fn declare(&mut self, kind: &str, name: &str, scope: Scope) {
        let normalized;
        let name = if Type::from_name(kind).is_some_and(|ty| config_kind(ty).is_some())
            && !matches!(kind, "varp" | "varbit" | "varn" | "vars")
        {
            normalized = crate::symbol::normalize_entity_name(name);
            normalized.as_ref()
        } else {
            name
        };
        *self
            .owners
            .entry(kind.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default() |= scope as u8;
    }

    pub(crate) fn check(
        &self,
        path: &Path,
        kind: &str,
        name: &str,
        required: bool,
    ) -> Option<String> {
        if !self.active {
            return None;
        }
        let scope = self.scope(path)?;
        let name = if matches!(kind, "varp" | "varbit" | "varn" | "vars") {
            name.trim_start_matches('.')
        } else {
            name
        };
        let name = if kind == "dbtable" {
            name.split(':').next().unwrap_or(name)
        } else {
            name
        };
        let owners = self
            .owners
            .get(kind)
            .and_then(|names| names.get(name))
            .copied()
            .unwrap_or(0);
        if (kind == "constant" || kind == "proc")
            && owners & Scope::Global as u8 != 0
            && owners & (Scope::Rune as u8 | Scope::Client as u8) != 0
        {
            return Some(format!(
                "{kind} '{name}' is declared in both globalscript and a private namespace; keep the shared declaration in globalscript"
            ));
        }
        if owners == 0 && !required {
            return None;
        }
        if owners & (scope as u8 | Scope::Global as u8) != 0 {
            return None;
        }
        let allowed = if scope == Scope::Global {
            "scripts/globalscript".to_owned()
        } else {
            format!("scripts/{} or scripts/globalscript", scope.folder())
        };
        Some(format!(
            "{kind} '{name}' is not visible to {}; define it in {allowed}",
            scope.folder()
        ))
    }

    pub(crate) fn typed(
        &self,
        path: &Path,
        name: &str,
        ty: Type,
        registry: &SymbolRegistry,
    ) -> Option<String> {
        if let Some(kind) = config_kind(ty) {
            let normalized;
            let name = if !matches!(ty, Type::Varp | Type::Varbit | Type::Varn | Type::Vars) {
                normalized = crate::symbol::normalize_entity_name(name);
                normalized.as_ref()
            } else {
                name
            };
            return self.check(path, kind, name, true);
        }
        match ty {
            Type::Proc
            | Type::Label
            | Type::Queue
            | Type::Timer
            | Type::SoftTimer
            | Type::Walktrigger => self.check(path, ty.name(), name, true),
            Type::ClientScript => self.check(
                path,
                if registry
                    .lookup_script_by_trigger("clientscript", name)
                    .is_some()
                {
                    "clientscript"
                } else {
                    "proc"
                },
                name,
                true,
            ),
            _ => None,
        }
    }

    fn numeric(&self, path: &Path, id: i32, ty: Type) -> Option<String> {
        // Negative sentinels and event placeholders are values, not config IDs.
        if id < 0 {
            return None;
        }
        let kind = config_kind(ty)?;
        self.check(
            path,
            kind,
            self.ids
                .get(&(ty, id))
                .map(String::as_str)
                .unwrap_or(&id.to_string()),
            true,
        )
    }

    pub(crate) fn reference(
        &self,
        path: &Path,
        expr: &Expr,
        hint: Option<Type>,
        registry: &SymbolRegistry,
    ) -> Option<String> {
        if !self.active {
            return None;
        }
        match expr {
            Expr::GameVar(name, _) => {
                let kind = match registry.lookup_game_var(name).map(|s| &s.kind) {
                    Some(SymbolKind::GameVar { category, .. }) => category.as_str(),
                    _ => return None,
                };
                self.check(path, kind, name, true)
            }
            Expr::ConstantVar(name, _) => self.check(path, "constant", name, false).or_else(|| {
                if let Some(SymbolKind::Constant {
                    int_value: Some(id),
                    ..
                }) = registry.lookup_constant(name).map(|s| &s.kind)
                {
                    self.numeric(path, *id, hint?)
                } else {
                    None
                }
            }),
            Expr::IntLiteral(id) => self.numeric(path, *id, hint?),
            Expr::StringLiteral(name) => self.typed(path, name, hint?, registry),
            Expr::Identifier(name) => {
                if let Some(ty) = hint
                    && registry.lookup_reference(name, ty).is_some()
                {
                    return self.typed(path, name, ty, registry);
                }
                if registry.lookup_constant(name).is_some() {
                    self.reference(path, &Expr::ConstantVar(name.clone(), 0), hint, registry)
                } else if let Some(entity) = registry.lookup_entity_id(name) {
                    self.typed(path, name, entity.const_type, registry)
                } else if registry
                    .lookup_command(name.trim_start_matches('.'))
                    .is_some()
                    || registry.type_chars.contains_key(name)
                {
                    None
                } else {
                    self.typed(path, name, hint?, registry)
                }
            }
            Expr::ProcCall { name, .. } => self.check(
                path,
                if registry.lookup_script_by_trigger("proc", name).is_none()
                    && registry
                        .lookup_script_by_trigger("clientscript", name)
                        .is_some()
                {
                    "clientscript"
                } else {
                    "proc"
                },
                name,
                true,
            ),
            Expr::JumpCall { name, .. } => self.check(path, "label", name, true),
            Expr::BinaryOp {
                op: crate::parser::BinOp::Add,
                lhs,
                rhs,
            } => {
                if let (Expr::Identifier(left), Expr::Identifier(right)) =
                    (lhs.as_ref(), rhs.as_ref())
                {
                    let name = format!("{left}+{right}");
                    if let Some(entity) = registry.lookup_entity_id(&name) {
                        return self.typed(
                            path,
                            &name,
                            hint.unwrap_or(entity.const_type),
                            registry,
                        );
                    }
                }
                None
            }
            _ => None,
        }
    }
}
