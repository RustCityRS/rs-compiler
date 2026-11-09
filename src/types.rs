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
    Colour,
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
    Vorbis,
    Midi,
    Jingle,
    Graphic,
    FontMetrics,
    ChatCat,
    ChatPhrase,
    Model,
    Texture,
    IdKit,
    MesAnim,
    Hitmark,

    // UI types
    Stat,
    Component,
    IfType,
    SetPosH,
    SetPosV,
    SetSize,
    SetTextAlignH,
    SetTextAlignV,
    ClanForumQfc,
    /// A parameter holding the number of one of the caller's arrays.
    IntArray,
    Key,
    ChatType,
    ChatFilter,
    WindowMode,
    PlatformType,
    IntBool,
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
    Varc,
    VarClan,
    VarClanSetting,
    VarCon,
    VarConBit,
    VarNBit,
    VarObj,

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

    // Config tables
    Bas,
    Billboard,
    ParticleEffector,
    ParticleEmitter,
    Cursor,
    Flo,
    Flu,
    ItemCode,
    Light,
    Msi,
    Quest,

    // Controller type (instanced area manager)
    Controller,
    RegionCoord,
    /// A region (instance), as `region_uid` gives it and
    /// `region_findbyuid` takes it.
    RegionUid,

    // Script reference types (for parameters that hold script IDs)
    Proc,
    ClientScript,
    Label,
    Queue,
    SoftTimer,
    Timer,
    Walktrigger,
    ControllerQueue,
    NpcQueue,

    // Meta types (used internally by the compiler)
    Any,
    Error,
    Void,
}

impl Type {
    pub fn to_char(self) -> char {
        // A clan forum link is a long to the client.
        if self == Self::ClanForumQfc {
            return '§';
        }
        Self::CHARS
            .chars()
            .find(|&ch| Self::from_char(ch) == Some(self))
            .unwrap_or('i')
    }

    /// Every character a script names a type by.
    pub const CHARS: &'static str = "is§1clnoOpNAtPMjdmxKSIavgJyVÐþ`µzf«ke@\u{80}";

    pub fn from_char(ch: char) -> Option<Self> {
        Some(match ch {
            'i' => Self::Int,
            's' => Self::String,
            '§' => Self::Long,
            'z' => Self::Char,
            'f' => Self::FontMetrics,
            '«' => Self::Vorbis,
            'k' => Self::ChatCat,
            'e' => Self::ChatPhrase,
            '1' => Self::Boolean,
            'c' => Self::Coord,
            'l' => Self::Loc,
            'n' => Self::Npc,
            'o' => Self::Obj,
            'O' => Self::NamedObj,
            'p' => Self::PlayerUid,
            'N' => Self::NpcUid,
            'A' => Self::Seq,
            't' => Self::Spotanim,
            'P' => Self::Synth,
            'M' => Self::Midi,
            'j' => Self::Jingle,
            'd' => Self::Graphic,
            'm' => Self::Model,
            'x' => Self::Texture,
            'K' => Self::IdKit,
            'S' => Self::Stat,
            'I' => Self::Component,
            'a' => Self::Interface,
            'v' => Self::Inv,
            'g' => Self::Enum,
            'J' => Self::Struct,
            'y' => Self::Category,
            'V' => Self::Varp,
            'Ð' => Self::DbRow,
            'þ' => Self::NpcStat,
            '`' => Self::MapArea,
            'µ' => Self::MapElement,
            '@' => Self::Cursor,
            '\u{80}' => Self::Bas,
            _ => return None,
        })
    }

    /// Returns the base var type (determines which stack to use).
    pub fn base_type(&self) -> BaseVarType {
        match self {
            Type::String => BaseVarType::String,
            Type::Long | Type::ClanForumQfc => BaseVarType::Long,
            _ => BaseVarType::Integer,
        }
    }

    /// Default value for uninitialised local variable declarations.
    /// Boolean and Int default to 0; all other int-based types default to -1 (null).
    pub fn default_int_value(&self) -> i32 {
        match self {
            Type::Int | Type::Boolean => 0,
            ty if ty.is_int_kind() => 0,
            _ => -1,
        }
    }

    /// Default value for implicit return values at end of scripts.
    /// Only Int pushes 0; everything else (including Boolean) pushes -1.
    pub fn default_return_value(&self) -> i32 {
        match self {
            Type::Int => 0,
            ty if ty.is_int_kind() => 0,
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
            "colour" | "color" | "rgb" => Some(Type::Colour),
            "clanforumqfc" => Some(Type::ClanForumQfc),
            "intarray" => Some(Type::IntArray),
            "key" => Some(Type::Key),
            "chattype" => Some(Type::ChatType),
            "chatfilter" => Some(Type::ChatFilter),
            "windowmode" => Some(Type::WindowMode),
            "platformtype" => Some(Type::PlatformType),
            "intbool" => Some(Type::IntBool),
            "iftype" => Some(Type::IfType),
            "setposh" => Some(Type::SetPosH),
            "setposv" => Some(Type::SetPosV),
            "setsize" => Some(Type::SetSize),
            "settextalignh" => Some(Type::SetTextAlignH),
            "settextalignv" => Some(Type::SetTextAlignV),
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
            "vorbis" => Some(Type::Vorbis),
            "midi" => Some(Type::Midi),
            "jingle" => Some(Type::Jingle),
            "graphic" => Some(Type::Graphic),
            "fontmetrics" => Some(Type::FontMetrics),
            "chatcat" | "quickchatcat" => Some(Type::ChatCat),
            "chatphrase" | "quickchatphrase" => Some(Type::ChatPhrase),
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
            "varpbit" => Some(Type::Varbit),
            "varc" => Some(Type::Varc),
            "varclan" => Some(Type::VarClan),
            "varclansetting" => Some(Type::VarClanSetting),
            "varcon" => Some(Type::VarCon),
            "varconbit" => Some(Type::VarConBit),
            "varnbit" => Some(Type::VarNBit),
            "varobj" => Some(Type::VarObj),
            "mapzone" => Some(Type::MapZone),
            "wma" | "maparea" => Some(Type::MapArea),
            "mapelement" | "mel" => Some(Type::MapElement),
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
            "bas" => Some(Type::Bas),
            "billboard" => Some(Type::Billboard),
            "particleeffector" => Some(Type::ParticleEffector),
            "particleemitter" => Some(Type::ParticleEmitter),
            "cursor" => Some(Type::Cursor),
            "flo" => Some(Type::Flo),
            "flu" => Some(Type::Flu),
            "itemcode" => Some(Type::ItemCode),
            "light" => Some(Type::Light),
            "msi" => Some(Type::Msi),
            "quest" => Some(Type::Quest),
            "regioncoord" => Some(Type::RegionCoord),
            "regionuid" | "region_uid" => Some(Type::RegionUid),
            // Script reference types (used when passing proc/label/queue references as params)
            "label" => Some(Type::Label),
            "proc" => Some(Type::Proc),
            "clientscript" => Some(Type::ClientScript),
            "command" => Some(Type::Int),
            "queue" => Some(Type::Queue),
            "softtimer" => Some(Type::SoftTimer),
            "timer" => Some(Type::Timer),
            "controller_queue" => Some(Type::ControllerQueue),
            "npc_queue" => Some(Type::NpcQueue),
            "walktrigger" => Some(Type::Walktrigger),
            "any" => Some(Type::Any),
            "intparam" | "stringparam" => Some(Type::Param),
            "type" => Some(Type::Int),
            _ => None,
        }
    }

    /// An int that only differs in how its values print: declarations say `int`.
    pub fn is_int_kind(&self) -> bool {
        matches!(
            self,
            Type::Colour
                | Type::Key
                | Type::ChatType
                | Type::ChatFilter
                | Type::WindowMode
                | Type::PlatformType
                | Type::IntBool
                | Type::IfType
                | Type::SetPosH
                | Type::SetPosV
                | Type::SetSize
                | Type::SetTextAlignH
                | Type::SetTextAlignV
        )
    }

    /// The name a declaration of this type gives.
    pub fn declared_name(&self) -> &'static str {
        if self.is_int_kind() {
            "int"
        } else {
            self.name()
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
            Type::Colour => "rgb",
            Type::ClanForumQfc => "clanforumqfc",
            Type::IntArray => "intarray",
            Type::Key => "key",
            Type::ChatType => "chattype",
            Type::ChatFilter => "chatfilter",
            Type::WindowMode => "windowmode",
            Type::PlatformType => "platformtype",
            Type::IntBool => "intbool",
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
            Type::Vorbis => "vorbis",
            Type::Midi => "midi",
            Type::Jingle => "jingle",
            Type::Graphic => "graphic",
            Type::FontMetrics => "fontmetrics",
            Type::ChatCat => "chatcat",
            Type::ChatPhrase => "chatphrase",
            Type::Model => "model",
            Type::Texture => "texture",
            Type::IdKit => "idkit",
            Type::MesAnim => "mesanim",
            Type::Hitmark => "hitmark",
            Type::Stat => "stat",
            Type::Component => "component",
            Type::IfType => "iftype",
            Type::SetPosH => "setposh",
            Type::SetPosV => "setposv",
            Type::SetSize => "setsize",
            Type::SetTextAlignH => "settextalignh",
            Type::SetTextAlignV => "settextalignv",
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
            Type::Varbit => "varpbit",
            Type::Varc => "varc",
            Type::VarClan => "varclan",
            Type::VarClanSetting => "varclansetting",
            Type::VarCon => "varcon",
            Type::VarConBit => "varconbit",
            Type::VarNBit => "varnbit",
            Type::VarObj => "varobj",
            Type::MapZone => "mapzone",
            Type::MapArea => "wma",
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
            Type::Bas => "bas",
            Type::Billboard => "billboard",
            Type::ParticleEffector => "particleeffector",
            Type::ParticleEmitter => "particleemitter",
            Type::Cursor => "cursor",
            Type::Flo => "flo",
            Type::Flu => "flu",
            Type::ItemCode => "itemcode",
            Type::Light => "light",
            Type::Msi => "msi",
            Type::Quest => "quest",
            Type::RegionCoord => "regioncoord",
            Type::RegionUid => "regionuid",
            Type::Proc => "proc",
            Type::ClientScript => "clientscript",
            Type::Label => "label",
            Type::Queue => "queue",
            Type::SoftTimer => "softtimer",
            Type::Timer => "timer",
            Type::ControllerQueue => "controller_queue",
            Type::NpcQueue => "npc_queue",
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
            "def_varpbit",
            "def_varc",
            "def_varclan",
            "def_varclansetting",
            "def_varcon",
            "def_varconbit",
            "def_varnbit",
            "def_varobj",
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
            "def_controller",
            "def_regionuid",
            "def_bas",
            "def_billboard",
            "def_effector",
            "def_emitter",
            "def_cursor",
            "def_flo",
            "def_flu",
            "def_itemcode",
            "def_light",
            "def_msi",
            "def_quest",
        ]
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
