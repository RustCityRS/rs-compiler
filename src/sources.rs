//! Source discovery and visibility independent of developer-chosen folders.
use crate::parallel::parallel_map;
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

    pub fn extension(self) -> &'static str {
        match self {
            Self::Rune => "rs2",
            Self::Client => "cs2",
            Self::Global => "gs2",
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "rs2" => Some(Self::Rune),
            "cs2" => Some(Self::Client),
            "gs2" => Some(Self::Global),
            _ => None,
        }
    }

    /// Scripts use their extension. Configs and constants default to shared;
    /// a leading `// @scope(server|client|shared)` comment can restrict them.
    pub fn for_file(path: &Path, text: &str) -> io::Result<Self> {
        if let Some(scope) = Self::from_path(path) {
            return Ok(scope);
        }
        let mut scope = None;
        for line in text.lines() {
            let line = line.trim().trim_start_matches('\u{feff}');
            if line.is_empty() {
                continue;
            }
            let Some(comment) = line.strip_prefix("//") else {
                break;
            };
            let Some(value) = comment.trim().strip_prefix("@scope") else {
                continue;
            };
            let value = value
                .trim()
                .strip_prefix('(')
                .and_then(|v| v.strip_suffix(')'));
            let declared = match value.map(str::trim) {
                Some("server") => Self::Rune,
                Some("client") => Self::Client,
                Some("shared") => Self::Global,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{}: expected // @scope(server|client|shared)",
                            path.display()
                        ),
                    ));
                }
            };
            if scope.replace(declared).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: duplicate @scope declaration", path.display()),
                ));
            }
        }
        Ok(scope.unwrap_or(Self::Global))
    }
}

/// A revision's scripts tree, including when a caller supplies an old target
/// subdirectory. Standalone projects can use any explicitly supplied root.
pub fn source_root(dir: &Path) -> PathBuf {
    dir.ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "scripts"))
        .unwrap_or(dir)
        .to_path_buf()
}

pub(crate) fn target_dir(dir: &Path, _scope: Scope) -> PathBuf {
    source_root(dir)
}

/// All targets share the same root; extensions select their script sources.
pub fn roots(dir: &Path) -> Vec<PathBuf> {
    vec![source_root(dir)]
}

pub fn config_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in roots(dir) {
        collect_all_files(&root, &mut files);
    }
    files.sort();
    files
}

fn file_scope(path: &Path) -> io::Result<Scope> {
    if let Some(scope) = Scope::from_path(path) {
        return Ok(scope);
    }
    let mut header = String::new();
    for line in io::BufReader::new(fs::File::open(path)?).lines() {
        let line = line?;
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if !trimmed.is_empty() && !trimmed.starts_with("//") {
            break;
        }
        header.push_str(&line);
        header.push('\n');
    }
    Scope::for_file(path, &header)
}

/// Read only the configs a target can use. Ownership indexing still sees all
/// declarations, so opposite-target references receive visibility errors.
pub fn target_config_files(dir: &Path, target: Scope) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in config_files(dir) {
        if crate::filesystem::config_kind(&path).is_none() {
            continue;
        }
        let scope = file_scope(&path)?;
        if scope == Scope::Global || scope == target {
            files.push(path);
        }
    }
    Ok(files)
}

pub fn script_files(dir: &Path, extension: &str) -> io::Result<Vec<PathBuf>> {
    let root = source_root(dir);
    // Report a missing source root instead of silently compiling nothing.
    fs::read_dir(&root)?;
    let mut files = config_files(&root);
    files.retain(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == extension || ext == "gs2")
    });
    Ok(files)
}

/// Locate an engine declaration by its filename, wherever it is organized.
pub fn engine_file(dir: &Path, scope: Scope) -> io::Result<Option<PathBuf>> {
    let name = format!("engine.{}", scope.extension());
    let mut matches = config_files(dir)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|file| file == name.as_str()));
    let first = matches.next();
    if let Some(second) = matches.next() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Multiple {name} files: {} and {}",
                first.as_ref().unwrap().display(),
                second.display(),
            ),
        ));
    }
    Ok(first)
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

/// The `(kind, name)` pairs a source or config file declares.
fn declarations(file: &Path) -> Vec<(String, String)> {
    let mut declared = Vec::new();
    let Some(ext) = file.extension().and_then(|ext| ext.to_str()) else {
        return declared;
    };
    let is_script = matches!(ext, "rs2" | "cs2" | "gs2");
    if !is_script
        && ext != "constant"
        && !Type::from_name(ext).is_some_and(|ty| config_kind(ty).is_some())
    {
        return declared;
    }
    let Ok(input) = fs::File::open(file) else {
        return declared;
    };
    let mut declare = |kind: &str, name: &str| declared.push((kind.to_owned(), name.to_owned()));
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
            if let Some(header) = line.strip_prefix('[').and_then(|line| line.split_once(']'))
                && let Some((trigger, name)) = header.0.split_once(',')
                && trigger.trim() != "command"
            {
                declare(trigger.trim(), name.trim());
            }
        } else if ext == "constant" {
            if let Some((name, _)) = line.split_once('=') {
                declare("constant", name.trim().trim_start_matches('^'));
            }
        } else if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            if ext == "varc" {
                declare("varc", name);
                declare("varcstr", name);
            } else if ext == "varclansetting" {
                for kind in [
                    "varclansetting",
                    "clansetting_int",
                    "clansetting_varbit",
                    "clansetting_long",
                    "clansetting_string",
                ] {
                    declare(kind, name);
                }
            } else {
                declare(ext, name);
            }
        }
    }
    declared
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
        | Type::Hitmark
        | Type::Area
        | Type::Bas
        | Type::Billboard
        | Type::ParticleEffector
        | Type::ParticleEmitter
        | Type::Cursor
        | Type::Flo
        | Type::Flu
        | Type::ItemCode
        | Type::Light
        | Type::Msi
        | Type::Quest
        | Type::Varc
        | Type::VarClan
        | Type::VarClanSetting
        | Type::VarCon
        | Type::VarConBit
        | Type::VarNBit
        | Type::VarObj => Some(ty.name()),
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
            .or_else(|| Scope::from_path(label))
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
        let mut result = Self::scan(dir);
        result.index_registry(registry);
        result
    }

    /// What the sources and configs around `dir` declare, before
    /// [`index_registry`](Self::index_registry). The files are read in
    /// parallel and declared in their order.
    pub(crate) fn scan(dir: &Path) -> Self {
        let files: Vec<_> = config_files(dir)
            .into_iter()
            .filter(|path| {
                Scope::from_path(path).is_some() || crate::filesystem::config_kind(path).is_some()
            })
            .collect();
        let mut result = Self {
            active: true,
            ..Self::default()
        };
        for (file, scope, declared) in parallel_map(&files, |file| {
            (
                file.clone(),
                file_scope(file).unwrap_or(Scope::Global),
                declarations(file),
            )
        }) {
            result.bind(file, scope);
            for (kind, name) in declared {
                result.declare(&kind, &name, scope);
            }
        }
        result
    }

    /// Index typed numeric references after the caller has supplied config IDs.
    pub fn index_registry(&mut self, registry: &SymbolRegistry) {
        self.ids.clear();
        for (name, entry) in &registry.entity_ids {
            for &(ty, id) in &entry.variants {
                if config_kind(ty).is_some() || ty == Type::Graphic {
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
            && !matches!(kind, "varp" | "varpbit" | "varn" | "vars")
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
        let name = if matches!(
            kind,
            "varp"
                | "varpbit"
                | "varn"
                | "vars"
                | "varc"
                | "varcstr"
                | "varclan"
                | "varobj"
                | "varnbit"
                | "varcon"
                | "varconbit"
        ) {
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
                "{kind} '{name}' is declared in both shared and private scopes; keep one shared declaration"
            ));
        }
        if owners == 0 && !required {
            return None;
        }
        if owners & (scope as u8 | Scope::Global as u8) != 0 {
            return None;
        }
        let allowed = if scope == Scope::Global {
            "a .gs2 source or shared config".to_owned()
        } else {
            format!(
                "a .{} or .gs2 source, or a visible config",
                scope.extension()
            )
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
            | Type::Walktrigger
            | Type::ControllerQueue
            | Type::NpcQueue => self.check(path, ty.name(), name, true),
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

    pub(crate) fn config_name(&self, ty: Type, id: i32) -> Option<&str> {
        self.ids.get(&(ty, id)).map(String::as_str)
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
                } else if let Some(entity) = registry.lookup_entity_id(name)
                    && (entity.const_type != Type::Param
                        || hint == Some(Type::Param)
                        || !registry.type_chars.contains_key(name))
                {
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
