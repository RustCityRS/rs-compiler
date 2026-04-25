/// Base variable types in the RS2 type system.
/// Every ScriptVarType maps to one of these for stack operations.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum BaseVarType {
    Integer,
    String,
    Long,
}

/// All script variable types supported by RuneScript.
/// These correspond to the types used in the 2004Scape/RS2 engine.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Type {
    // Primitive types
    Int,
    Boolean,
    String,
    Long,
    Char,
    Coord,

    // Game entity types
    Loc,
    Npc,
    Obj,
    NamedObj,
    PlayerUid,
    NpcUid,

    // Visual/audio types
    Seq,
    Spotanim,
    Synth,
    Midi,
    Jingle,
    Graphic,
    FontMetrics,
    Model,
    Texture,
    IdKit,
    MesAnim,
    Hitmark,

    // UI types
    Stat,
    Component,
    Interface,
    TopLevelInterface,
    OverlayInterface,
    Inv,

    // Data types
    Enum,
    Struct,
    Param,
    DbTable,
    DbRow,
    DbColumn,
    Category,

    // Variable types
    Varp,
    Varn,
    Vars,
    Varbit,

    // Map types
    MapZone,
    MapArea,
    MapElement,

    // Misc types
    LocShape,
    NpcMode,
    NpcStat,
    Area,
    Hunt,
    MoveSpeed,
    EntityOverlay,
    StringVector,
    WriteinvObj,

    // Controller type (instanced area manager)
    Controller,
    RegionCoord,

    // Script reference types (for parameters that hold script IDs)
    Proc,
    Label,
    Queue,
    SoftTimer,
    Timer,
    Walktrigger,

    // Meta types (used internally by the compiler)
    Any,
    Error,
    Void,
}

impl Type {
    /// Returns the base var type (determines which stack to use).
    pub fn base_type(&self) -> BaseVarType {
        match self {
            Type::String => BaseVarType::String,
            Type::Long => BaseVarType::Long,
            _ => BaseVarType::Integer,
        }
    }

    /// Default value for uninitialised local variable declarations.
    /// Boolean and Int default to 0; all other int-based types default to -1 (null).
    pub fn default_int_value(&self) -> i32 {
        match self {
            Type::Int | Type::Boolean => 0,
            _ => -1,
        }
    }

    /// Default value for implicit return values at end of scripts.
    /// Only Int pushes 0; everything else (including Boolean) pushes -1.
    pub fn default_return_value(&self) -> i32 {
        match self {
            Type::Int => 0,
            _ => -1,
        }
    }

    /// Parse a type name from a def_ prefix (e.g., "def_int" -> Int).
    pub fn from_def_str(s: &str) -> Option<Type> {
        let type_name = s.strip_prefix("def_")?;
        Self::from_name(type_name)
    }

    /// Parse a type from its bare name (e.g., "int" -> Int).
    pub fn from_name(s: &str) -> Option<Type> {
        match s {
            "int" => Some(Type::Int),
            "boolean" | "bool" => Some(Type::Boolean),
            "string" => Some(Type::String),
            "long" => Some(Type::Long),
            "char" => Some(Type::Char),
            "coord" => Some(Type::Coord),
            "loc" => Some(Type::Loc),
            "npc" => Some(Type::Npc),
            "obj" => Some(Type::Obj),
            "namedobj" => Some(Type::NamedObj),
            "playeruid" | "player_uid" => Some(Type::PlayerUid),
            "npcuid" | "npc_uid" => Some(Type::NpcUid),
            "seq" => Some(Type::Seq),
            "spotanim" => Some(Type::Spotanim),
            "synth" => Some(Type::Synth),
            "midi" => Some(Type::Midi),
            "jingle" => Some(Type::Jingle),
            "graphic" => Some(Type::Graphic),
            "fontmetrics" => Some(Type::FontMetrics),
            "model" => Some(Type::Model),
            "texture" => Some(Type::Texture),
            "idk" | "idkit" => Some(Type::IdKit),
            "mesanim" => Some(Type::MesAnim),
            "hitmark" => Some(Type::Hitmark),
            "stat" => Some(Type::Stat),
            "component" => Some(Type::Component),
            "interface" => Some(Type::Interface),
            "toplevelinterface" => Some(Type::TopLevelInterface),
            "overlayinterface" => Some(Type::OverlayInterface),
            "inv" => Some(Type::Inv),
            "enum" => Some(Type::Enum),
            "struct" => Some(Type::Struct),
            "param" => Some(Type::Param),
            "dbtable" => Some(Type::DbTable),
            "dbrow" => Some(Type::DbRow),
            "dbcolumn" => Some(Type::DbColumn),
            "category" => Some(Type::Category),
            "varp" => Some(Type::Varp),
            "varn" => Some(Type::Varn),
            "vars" => Some(Type::Vars),
            "varbit" => Some(Type::Varbit),
            "mapzone" => Some(Type::MapZone),
            "maparea" => Some(Type::MapArea),
            "mapelement" => Some(Type::MapElement),
            "locshape" | "loc_shape" => Some(Type::LocShape),
            "npc_mode" | "npcmode" => Some(Type::NpcMode),
            "npc_stat" | "npcstat" => Some(Type::NpcStat),
            "area" => Some(Type::Area),
            "hunt" => Some(Type::Hunt),
            "movespeed" => Some(Type::MoveSpeed),
            "entityoverlay" => Some(Type::EntityOverlay),
            "stringvector" => Some(Type::StringVector),
            "writeinvobj" => Some(Type::WriteinvObj),
            "controller" => Some(Type::Controller),
            "regioncoord" => Some(Type::RegionCoord),
            // Script reference types (used when passing proc/label/queue references as params)
            "label" => Some(Type::Label),
            "proc" => Some(Type::Proc),
            "command" => Some(Type::Int),
            "queue" => Some(Type::Queue),
            "softtimer" => Some(Type::SoftTimer),
            "timer" => Some(Type::Timer),
            "walktrigger" => Some(Type::Walktrigger),
            "any" => Some(Type::Any),
            "intparam" | "stringparam" => Some(Type::Param),
            "type" => Some(Type::Int),
            _ => None,
        }
    }

    /// Get the name of this type as used in RuneScript source.
    pub fn name(&self) -> &'static str {
        match self {
            Type::Int => "int",
            Type::Boolean => "boolean",
            Type::String => "string",
            Type::Long => "long",
            Type::Char => "char",
            Type::Coord => "coord",
            Type::Loc => "loc",
            Type::Npc => "npc",
            Type::Obj => "obj",
            Type::NamedObj => "namedobj",
            Type::PlayerUid => "playeruid",
            Type::NpcUid => "npcuid",
            Type::Seq => "seq",
            Type::Spotanim => "spotanim",
            Type::Synth => "synth",
            Type::Midi => "midi",
            Type::Jingle => "jingle",
            Type::Graphic => "graphic",
            Type::FontMetrics => "fontmetrics",
            Type::Model => "model",
            Type::Texture => "texture",
            Type::IdKit => "idkit",
            Type::MesAnim => "mesanim",
            Type::Hitmark => "hitmark",
            Type::Stat => "stat",
            Type::Component => "component",
            Type::Interface => "interface",
            Type::TopLevelInterface => "toplevelinterface",
            Type::OverlayInterface => "overlayinterface",
            Type::Inv => "inv",
            Type::Enum => "enum",
            Type::Struct => "struct",
            Type::Param => "param",
            Type::DbTable => "dbtable",
            Type::DbRow => "dbrow",
            Type::DbColumn => "dbcolumn",
            Type::Category => "category",
            Type::Varp => "varp",
            Type::Varn => "varn",
            Type::Vars => "vars",
            Type::Varbit => "varbit",
            Type::MapZone => "mapzone",
            Type::MapArea => "maparea",
            Type::MapElement => "mapelement",
            Type::LocShape => "loc_shape",
            Type::NpcMode => "npc_mode",
            Type::NpcStat => "npc_stat",
            Type::Area => "area",
            Type::Hunt => "hunt",
            Type::MoveSpeed => "movespeed",
            Type::EntityOverlay => "entityoverlay",
            Type::StringVector => "stringvector",
            Type::WriteinvObj => "writeinvobj",
            Type::Controller => "controller",
            Type::RegionCoord => "regioncoord",
            Type::Proc => "proc",
            Type::Label => "label",
            Type::Queue => "queue",
            Type::SoftTimer => "softtimer",
            Type::Timer => "timer",
            Type::Walktrigger => "walktrigger",
            Type::Any => "any",
            Type::Error => "error",
            Type::Void => "void",
        }
    }

    pub fn allow_switch(&self) -> bool {
        self.base_type() == BaseVarType::Integer
    }

    /// Returns true if this is a numeric type that supports arithmetic.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Long)
    }

    /// Returns true if this is a type that uses object-style comparison (string equality).
    pub fn is_obj_type(&self) -> bool {
        matches!(self, Type::String)
    }

    /// Returns all known def_ type names.
    pub fn all_def_keywords() -> Vec<&'static str> {
        vec![
            "def_int",
            "def_boolean",
            "def_string",
            "def_long",
            "def_char",
            "def_coord",
            "def_loc",
            "def_npc",
            "def_obj",
            "def_namedobj",
            "def_playeruid",
            "def_npcuid",
            "def_seq",
            "def_spotanim",
            "def_synth",
            "def_midi",
            "def_jingle",
            "def_graphic",
            "def_fontmetrics",
            "def_model",
            "def_texture",
            "def_idkit",
            "def_mesanim",
            "def_hitmark",
            "def_stat",
            "def_component",
            "def_interface",
            "def_toplevelinterface",
            "def_overlayinterface",
            "def_inv",
            "def_enum",
            "def_struct",
            "def_param",
            "def_dbtable",
            "def_dbrow",
            "def_dbcolumn",
            "def_category",
            "def_varp",
            "def_varn",
            "def_vars",
            "def_varbit",
            "def_mapzone",
            "def_maparea",
            "def_mapelement",
            "def_loc_shape",
            "def_npc_mode",
            "def_npc_stat",
            "def_area",
            "def_hunt",
            "def_movespeed",
            "def_entityoverlay",
            "def_stringvector",
            "def_writeinvobj",
        ]
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
