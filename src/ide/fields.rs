//! What each config file's keys hold, so a value resolves by its type the way
//! a script's argument does. Parts that take their type from a declaration
//! (`Input`, `Output`, `Declared`, `OfParam`, `OfColumn`) become a real
//! type in `slot`.

use super::colors::Encoding;
use crate::types::Type;
use Value::{
    AnyVar, Color, ColumnName, Component, Config, Declared, Flag, Floor, Hook, Input, Number,
    OfColumn, OfParam, Output, Sound, Text, TextureColor, Tile, TypeName, Word,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Config(Type),
    AnyVar,
    Number,
    Color(Encoding),
    TextureColor(&'static str),
    Flag,
    Word,
    Text,
    TypeName,
    Input,
    Output,
    Declared,
    OfParam,
    ColumnName,
    OfColumn,
    /// A synth or a vorbis.
    Sound,
    /// A component of the interface the file is.
    Component,
    /// A script, what it's given, and what it runs on.
    Hook,
    /// A world map tile: `underlay:name`, `overlay:name@index`, `layers:count,options` or a word.
    Tile,
    /// The underlay or overlay the tile before it names directly.
    Floor,
}

/// What a file has already declared that later values depend on.
#[derive(Default)]
pub(crate) struct ConfigTypes {
    pub(super) params: HashMap<String, Type>,
    pub(super) columns: HashMap<String, Vec<Type>>,
}

/// The keys of the section being read that type its other keys.
#[derive(Clone, Copy, Default)]
pub(super) struct Section<'a> {
    input: Option<&'a str>,
    output: Option<&'a str>,
    declared: Option<&'a str>,
    pub(super) table: Option<&'a str>,
    floor: Option<&'a str>,
}

impl<'a> Section<'a> {
    pub(super) fn note(&mut self, key: &str, value: &'a str) {
        match key {
            "inputtype" => self.input = Some(value),
            "outputtype" => self.output = Some(value),
            "type" => self.declared = Some(value),
            "table" => self.table = Some(value),
            _ if key.starts_with("tile") => self.floor = value.split(':').next(),
            _ => {}
        }
    }
}

type Field = (&'static str, &'static [Value]);

const NUMBER: &[Value] = &[Number];
const RGB: &[Value] = &[Color(Encoding::Rgb)];
const ARGB: &[Value] = &[Color(Encoding::Argb)];
const RGB15: &[Value] = &[Color(Encoding::Rgb15)];
const HSL: &[Value] = &[Color(Encoding::Hsl)];
const TEXTURE_COLOR: &[Value] = &[TextureColor("constant_color")];
const TEXTURE_TARGET_COLOR: &[Value] = &[TextureColor("color_replace")];
const FLAG: &[Value] = &[Flag];
const WORD: &[Value] = &[Word];
const TEXT: &[Value] = &[Text];
const TYPE: &[Value] = &[TypeName];
const CATEGORY: &[Value] = &[Config(Type::Category)];
const GRAPHIC: &[Value] = &[Config(Type::Graphic)];
const MODEL: &[Value] = &[Config(Type::Model)];
const NPC: &[Value] = &[Config(Type::Npc)];
const OBJ: &[Value] = &[Config(Type::Obj)];
const SEQ: &[Value] = &[Config(Type::Seq)];
const VARP: &[Value] = &[Config(Type::Varp)];
const WEIGHTED_SEQ: &[Value] = &[Config(Type::Seq), Number];
const OP_CURSOR: &[Value] = &[Number, Config(Type::Cursor)];
const QUEST: &[Value] = &[Config(Type::Quest)];
const TEXTURE: &[Value] = &[Config(Type::Texture)];
const CHATPHRASE: &[Value] = &[Config(Type::ChatPhrase)];
const CURSOR_ONLY: &[Value] = &[Config(Type::Cursor)];
const HOOK: &[Value] = &[Hook];
const PARAM: &[Value] = &[Config(Type::Param), OfParam];
const PARAM_INT: &[Value] = &[Config(Type::Param), Number];
const PARAM_TEXT: &[Value] = &[Config(Type::Param), Text];
const VARP_RANGE: &[Value] = &[Config(Type::Varp), Number, Number];
const VARBIT_RANGE: &[Value] = &[Config(Type::Varbit), Number, Number];
const TILE: &[Value] = &[Tile];

const AREA: &[Field] = &[("colour", RGB), ("trigger", FLAG)];

const BAS: &[Field] = &[
    ("readyanim", SEQ),
    ("readyanim_l", SEQ),
    ("readyanim_r", SEQ),
    ("readyanim_random", WEIGHTED_SEQ),
    ("walkanim", SEQ),
    ("walkanim_b", SEQ),
    ("walkanim_l", SEQ),
    ("walkanim_r", SEQ),
    ("runanim", SEQ),
    ("runanim_b", SEQ),
    ("runanim_l", SEQ),
    ("runanim_r", SEQ),
    ("crawlanim", SEQ),
    ("crawlanim_b", SEQ),
    ("crawlanim_l", SEQ),
    ("crawlanim_r", SEQ),
    ("rotate", NUMBER),
    ("turnspeed", NUMBER),
    ("turnmax", NUMBER),
    ("rollspeed", NUMBER),
    ("rollmax", NUMBER),
    ("roll", NUMBER),
    ("pitchspeed", NUMBER),
    ("pitchmax", NUMBER),
    ("pitch", NUMBER),
    ("slotoffset", NUMBER),
    ("op", NUMBER),
    ("noshadow", FLAG),
    ("randomreadyanim", WEIGHTED_SEQ),
    ("walkturn_l", SEQ),
    ("walkturn_r", SEQ),
    ("runturn_l", SEQ),
    ("runturn_r", SEQ),
    ("crawlturn_l", SEQ),
    ("crawlturn_r", SEQ),
    ("hillrotate", NUMBER),
    ("hillrotatelimit", NUMBER),
    ("unknown", NUMBER),
];

const BILLBOARD: &[Field] = &[
    ("material", &[Config(Type::Texture)]),
    ("size_minus_one", NUMBER),
    ("unused", NUMBER),
    ("blend_src", NUMBER),
    ("blend_dst", NUMBER),
    ("bloom_disable", FLAG),
    ("skip_triangle", FLAG),
    ("size", NUMBER),
    ("sourceblendmode", NUMBER),
    ("destinationblendmode", NUMBER),
    ("unknown", NUMBER),
];

const PARTICLEEFFECTOR: &[Field] = &[
    ("angle", NUMBER),
    ("falloff", NUMBER),
    ("forcedirection", NUMBER),
    ("forcetype", NUMBER),
    ("unknown", NUMBER),
];

const EFFECTORS: &[Value] = &[Config(Type::ParticleEffector)];

const PARTICLEEMITTER: &[Field] = &[
    ("alphatransitionpercent", NUMBER),
    ("coneangle", NUMBER),
    ("emissionrate", NUMBER),
    ("enabled", FLAG),
    ("killabovelevel", NUMBER),
    ("killbelowlevel", NUMBER),
    ("lifetime", NUMBER),
    ("lighting", FLAG),
    ("linkcolourchannels", FLAG),
    ("localeffectors", EFFECTORS),
    ("worldeffectors", EFFECTORS),
    ("material", TEXTURE),
    ("rgba", ARGB),
    ("rgbatarget", ARGB),
    ("rgbtransitionpercent", NUMBER),
    ("size", NUMBER),
    ("sizetarget", NUMBER),
    ("sizetargetpercent", NUMBER),
    ("speed", NUMBER),
    ("speedcontrol", NUMBER),
    ("speedtarget", NUMBER),
    ("speedtargetpercent", NUMBER),
    ("unknown", NUMBER),
];

const CURSOR: &[Field] = &[("sprite", GRAPHIC), ("hotspot", NUMBER)];

const DBROW: &[Field] = &[
    ("table", &[Config(Type::DbTable)]),
    ("data", &[ColumnName, OfColumn]),
    ("graphic", GRAPHIC),
];

const DBTABLE: &[Field] = &[("column", &[Word, TypeName])];

const ENUM: &[Field] = &[
    ("inputtype", TYPE),
    ("outputtype", TYPE),
    ("default", &[Output]),
    ("defaultint", &[Output]),
    ("defaultstring", TEXT),
    ("val", &[Input, Output]),
];

const FLO: &[Field] = &[
    ("color", RGB),
    ("water_color", RGB),
    ("texture", &[Config(Type::Texture)]),
    ("colour", RGB),
    ("mapcolour", RGB),
    ("material", TEXTURE),
    ("occlude", FLAG),
    ("priority", NUMBER),
    ("smoothedges", FLAG),
    ("texturescale", NUMBER),
    ("unknown", NUMBER),
    ("waterfogcolour", RGB),
    ("waterfogoffset", NUMBER),
    ("waterfogscale", NUMBER),
];

const FLU: &[Field] = &[
    ("color", RGB),
    ("size", NUMBER),
    ("texture", &[Config(Type::Texture)]),
    ("colour", RGB),
    ("hardshadow", FLAG),
    ("material", TEXTURE),
    ("occlude", FLAG),
    ("texturescale", NUMBER),
];

const HITMARK: &[Field] = &[
    ("classgraphic", GRAPHIC),
    ("leftgraphic", GRAPHIC),
    ("middlegraphic", GRAPHIC),
    ("rightgraphic", GRAPHIC),
    ("damagefont", &[Config(Type::FontMetrics)]),
    ("damagecolour", RGB),
    ("damageformat", TEXT),
    ("scrolltooffsety", NUMBER),
    ("fadeout", NUMBER),
    ("sticktime", NUMBER),
];

const HUNT: &[Field] = &[
    ("type", WORD),
    ("check_vis", WORD),
    ("check_nottoostrong", WORD),
    ("check_notbusy", WORD),
    ("check_afk", WORD),
    ("find_keephunting", WORD),
    ("nobodynear", WORD),
    ("find_newmode", &[Config(Type::NpcMode)]),
    ("check_notcombat", VARP),
    ("check_notcombat_self", &[Config(Type::Varn)]),
    ("rate", NUMBER),
    ("check_category", CATEGORY),
    ("check_npc", NPC),
    ("check_obj", OBJ),
    ("check_loc", &[Config(Type::Loc)]),
    ("check_inv", &[Config(Type::Inv), Config(Type::Obj), Word]),
    (
        "check_invparam",
        &[Config(Type::Inv), Config(Type::Param), Word],
    ),
    (
        "check_invcat",
        &[Config(Type::Inv), Config(Type::Category), Word],
    ),
    ("extracheck_var", &[AnyVar, Word]),
];

const IDK: &[Field] = &[
    ("model", MODEL),
    ("head", MODEL),
    ("type", WORD),
    ("disable", FLAG),
    ("bodypart", NUMBER),
];

const INV: &[Field] = &[
    ("stock", &[Config(Type::Obj), Number, Number]),
    ("size", NUMBER),
    ("scope", WORD),
    ("stackall", FLAG),
    ("restock", FLAG),
    ("allstock", FLAG),
    ("protect", FLAG),
    ("runweight", FLAG),
    ("dummyinv", FLAG),
];

const LIGHT: &[Field] = &[
    ("amplitude", NUMBER),
    ("speed", NUMBER),
    ("base", NUMBER),
    ("function", NUMBER),
    ("frequency", NUMBER),
    ("offset", NUMBER),
];

const LOC: &[Field] = &[
    ("model", &[Word, Config(Type::Model)]),
    ("model_sd", &[Number, Config(Type::Model)]),
    ("model_hd", &[Number, Config(Type::Model)]),
    ("multiloc", &[Number, Config(Type::Loc)]),
    ("multivar", &[AnyVar]),
    ("name", TEXT),
    ("desc", TEXT),
    ("op", TEXT),
    ("memberop", TEXT),
    ("anim", SEQ),
    ("random_anim", WEIGHTED_SEQ),
    ("category", CATEGORY),
    ("param", PARAM),
    ("paramint", PARAM_INT),
    ("paramstr", PARAM_TEXT),
    ("cursor", OP_CURSOR),
    ("msi_id", &[Config(Type::Msi)]),
    ("mapelement", &[Config(Type::MapElement)]),
    ("quest", QUEST),
    ("ambient_sfx", NUMBER),
    ("ambient_sfx_random", NUMBER),
    ("occlude", FLAG),
    ("occlude_off", FLAG),
    ("hillskew", FLAG),
    ("sharelight", FLAG),
    ("interact_type", FLAG),
    ("blockrange", FLAG),
    ("blockwalk", FLAG),
    ("forcedecor", FLAG),
    ("nominimap_visible", FLAG),
    ("mirror", FLAG),
    ("shadow", FLAG),
    ("breakroutefinding", FLAG),
    ("noblocks_npcs", FLAG),
    ("members_only", FLAG),
    ("hidden", FLAG),
    ("absent", FLAG),
    ("ambient", NUMBER),
    ("active", NUMBER),
    ("contrast", NUMBER),
    ("width", NUMBER),
    ("length", NUMBER),
    ("forceapproach", WORD),
    ("resizex", NUMBER),
    ("resizey", NUMBER),
    ("resizez", NUMBER),
    ("raiseobject", FLAG),
    ("hillskew_t", NUMBER),
    ("wallwidth", NUMBER),
    ("interaction_sides", NUMBER),
    ("offsetx", NUMBER),
    ("offsety", NUMBER),
    ("offsetz", NUMBER),
    ("modelsd", &[Word, Config(Type::Model)]),
    ("modelhd", &[Word, Config(Type::Model)]),
    ("membersop", TEXT),
    ("msi", &[Config(Type::Msi)]),
    ("msirotate", FLAG),
    ("members", FLAG),
    ("bgsound", &[Sound, Number]),
    ("bgsounddropoffrange", NUMBER),
    ("bgsoundrate", NUMBER),
    ("bgsoundvolume", NUMBER),
    ("randomsound", &[Number, Number, Number, Sound]),
    ("hardshadow", FLAG),
    ("hasparticles", FLAG),
    ("hillchange", &[Word, Number]),
    ("istexture", FLAG),
    ("raiseobjectgrounddecor", NUMBER),
    ("randomanimframe", FLAG),
    ("tint", NUMBER),
    ("unknown", NUMBER),
];

const MEL: &[Field] = &[
    ("sprite", GRAPHIC),
    ("mouseovergraphic", GRAPHIC),
    ("worldmaparrow", GRAPHIC),
    ("category", CATEGORY),
    ("text", TEXT),
    ("op", TEXT),
    ("opbase", TEXT),
    (
        "showvar",
        &[Config(Type::Varbit), Config(Type::Varp), Number, Number],
    ),
    ("show", WORD),
    ("mapfunction", FLAG),
    ("textsize", NUMBER),
    ("textcolour", RGB),
    ("textmouseovercolour", RGB),
    ("textbackgroundfill", ARGB),
    ("textbackgroundoutline", ARGB),
    ("polygonfill", ARGB),
    ("polygonoutline", ARGB),
    ("polygonpoint", NUMBER),
    ("condition", &[AnyVar, Number, Number]),
];

const MESANIM: &[Field] = &[("len", SEQ)];

const MSI: &[Field] = &[
    ("sprite", GRAPHIC),
    ("nosprite", FLAG),
    ("locsize", FLAG),
    ("colour", RGB),
    ("graphic", GRAPHIC),
    ("resize", FLAG),
];

const NPC_FIELDS: &[Field] = &[
    ("model", MODEL),
    ("head", MODEL),
    ("multi", NPC),
    ("multidefault", NPC),
    ("multivar", &[AnyVar]),
    ("multivarp", VARP),
    ("readyanim", SEQ),
    ("walkanim", SEQ),
    ("name", TEXT),
    ("desc", TEXT),
    ("op", TEXT),
    ("memberop", TEXT),
    ("param", PARAM),
    ("paramint", PARAM_INT),
    ("paramstr", PARAM_TEXT),
    ("category", CATEGORY),
    ("huntmode", &[Config(Type::Hunt)]),
    ("defaultmode", WORD),
    ("moverestrict", WORD),
    ("blockwalk", WORD),
    ("vislevel", NUMBER),
    ("cursor", OP_CURSOR),
    ("attackcursor", &[Config(Type::Cursor)]),
    ("alwaysontop", FLAG),
    ("minimap", FLAG),
    ("visiblepriority", FLAG),
    ("invisiblepriority", FLAG),
    ("animateidle", FLAG),
    ("clickable", FLAG),
    ("slowwalk", FLAG),
    ("members", FLAG),
    ("vorbis", FLAG),
    ("givechase", FLAG),
    ("hasalpha", FLAG),
    ("ambient", NUMBER),
    ("contrast", NUMBER),
    ("walkmask", NUMBER),
    ("size", NUMBER),
    ("mainoptionindex", NUMBER),
    ("resizeh", NUMBER),
    ("resizev", NUMBER),
    ("hitbar", NUMBER),
    ("turnspeed", NUMBER),
    ("wanderrange", NUMBER),
    ("maxrange", NUMBER),
    ("huntrange", NUMBER),
    ("attackrange", NUMBER),
    ("respawnrate", NUMBER),
    ("respawndir", NUMBER),
    ("regenrate", NUMBER),
    ("hitpoints", NUMBER),
    ("attack", NUMBER),
    ("defence", NUMBER),
    ("strength", NUMBER),
    ("magic", NUMBER),
    ("ranged", NUMBER),
    ("headicon", NUMBER),
    ("unknown", NUMBER),
    ("idlesound", NUMBER),
    ("crawlsound", NUMBER),
    ("walksound", NUMBER),
    ("runsound", NUMBER),
    ("sounddistance", NUMBER),
    ("soundvolume", NUMBER),
    ("campaign", NUMBER),
    ("timer", NUMBER),
    ("translate", NUMBER),
    ("slayertype", NUMBER),
    ("shadowcolour", NUMBER),
    ("shadowmod", NUMBER),
    ("tint", NUMBER),
    ("armyicon", NUMBER),
    ("height", NUMBER),
    ("mapfunction", NUMBER),
    ("patrol", NUMBER),
    ("multinpc", &[Number, Config(Type::Npc)]),
    ("membersop", TEXT),
    ("bas", &[Config(Type::Bas)]),
    ("cursorattack", CURSOR_ONLY),
    ("covermarker", GRAPHIC),
    ("defaultmovemode", WORD),
    ("familiar", FLAG),
    ("modeloffset", NUMBER),
    ("mapelement", &[Config(Type::MapElement)]),
    ("overlayheight", NUMBER),
    ("reprioritiseattackop", FLAG),
    ("roughbounding", NUMBER),
    ("spotshadow", FLAG),
    ("spotshadowcolour", HSL),
    ("spotshadowtrans", NUMBER),
    ("alwaysonbottom", FLAG),
    ("active", FLAG),
    ("bgsound", &[Sound, Sound, Sound, Sound, Number]),
    ("bgsoundvolume", NUMBER),
    ("walksmoothing", FLAG),
    ("quest", QUEST),
];

const OBJ_FIELDS: &[Field] = &[
    ("model", MODEL),
    ("manwear", MODEL),
    ("womanwear", MODEL),
    ("manhead", MODEL),
    ("womanhead", MODEL),
    ("certlink", OBJ),
    ("certtemplate", OBJ),
    ("lentlink", OBJ),
    ("lenttemplate", OBJ),
    ("boughtlink", OBJ),
    ("boughttemplate", OBJ),
    ("multistack", OBJ),
    ("count", &[Config(Type::Obj), Number]),
    ("code10", SEQ),
    ("category", CATEGORY),
    ("param", PARAM),
    ("paramint", PARAM_INT),
    ("paramstr", PARAM_TEXT),
    ("name", TEXT),
    ("desc", TEXT),
    ("op", TEXT),
    ("iop", TEXT),
    ("wearpos", WORD),
    ("weight", WORD),
    ("dummyitem", WORD),
    ("cursor", OP_CURSOR),
    ("icursor", OP_CURSOR),
    ("quest", QUEST),
    ("members", FLAG),
    ("stackable", FLAG),
    ("objvar", &[Config(Type::VarObj)]),
    ("tradeable", FLAG),
    ("stockmarket", FLAG),
    ("2dzoom", NUMBER),
    ("2dxan", NUMBER),
    ("2dyan", NUMBER),
    ("2dzan", NUMBER),
    ("2dxof", NUMBER),
    ("2dyof", NUMBER),
    ("cost", NUMBER),
    ("dummyobj", NUMBER),
    ("ambient", NUMBER),
    ("contrast", NUMBER),
    ("manwearoff", NUMBER),
    ("womanwearoff", NUMBER),
    ("team", NUMBER),
    ("resizex", NUMBER),
    ("resizey", NUMBER),
    ("resizez", NUMBER),
    ("respawnrate", NUMBER),
];

const PARAM_FIELDS: &[Field] = &[
    ("type", TYPE),
    ("default", &[Declared]),
    ("defaultint", &[Declared]),
    ("defaultstring", TEXT),
    ("autodisable", FLAG),
    ("client", WORD),
];

const QUEST_FIELDS: &[Field] = &[
    ("name", TEXT),
    ("sortname", TEXT),
    ("progressvarp", VARP_RANGE),
    ("progressvarbit", VARBIT_RANGE),
    ("varprequirement", VARP_RANGE),
    ("varbitrequirement", VARBIT_RANGE),
    ("questrequirement", QUEST),
    ("statrequirement", &[Config(Type::Stat), Number]),
    ("pointsrequirement", NUMBER),
    ("graphic", GRAPHIC),
    ("param", PARAM),
    ("points", NUMBER),
    ("difficulty", WORD),
    ("type", WORD),
    ("op", NUMBER),
    ("members", FLAG),
    ("icon", GRAPHIC),
    ("mainstartcoord", NUMBER),
    ("masterquestvarbit", VARBIT_RANGE),
    ("masterquestvarp", VARP_RANGE),
    ("parent", QUEST),
    ("questpoints", NUMBER),
    ("questreq", QUEST),
    ("statreq", &[Config(Type::Stat), Number]),
];

const SEQ_FIELDS: &[Field] = &[
    ("frame", WORD),
    ("iframe", WORD),
    ("delay", NUMBER),
    ("synth", NUMBER),
    ("op", NUMBER),
    ("sound_mask", NUMBER),
    ("loopoffset", NUMBER),
    ("priority", NUMBER),
    ("walkmerge", WORD),
    ("looptype", NUMBER),
    ("interrupt", NUMBER),
    ("loopcount", NUMBER),
    ("tween", FLAG),
    ("loops", NUMBER),
    ("maxloops", NUMBER),
    ("duplicatebehaviour", WORD),
    ("postanim_move", WORD),
    ("preanim_move", WORD),
    ("replaceheldleft", OBJ),
    ("replaceheldright", OBJ),
    ("sound", &[Sound, Number]),
    ("soundrandomrate", NUMBER),
    ("soundvolume", NUMBER),
    ("unknown", NUMBER),
];

const SPOTANIM: &[Field] = &[
    ("model", MODEL),
    ("anim", SEQ),
    ("ambient", NUMBER),
    ("contrast", NUMBER),
    ("resizeh", NUMBER),
    ("resizev", NUMBER),
    ("angle", NUMBER),
    ("allowloop", FLAG),
    ("rotation", NUMBER),
];

const STRUCT: &[Field] = &[
    ("param", PARAM),
    ("paramint", PARAM_INT),
    ("paramstr", PARAM_TEXT),
];

const QUICKCHATCAT: &[Field] = &[
    ("desc", TEXT),
    ("phrase", &[Config(Type::ChatPhrase), Number]),
    ("subcat", &[Config(Type::ChatCat), Number]),
    ("unsearchable", FLAG),
];

const QUICKCHATPHRASE: &[Field] = &[("autoresponse", CHATPHRASE), ("phrase", TEXT)];

const DEFAULTS: &[Field] = &[
    ("maxhitmarks", NUMBER),
    ("hitmarkoffset", NUMBER),
    ("performancemetricsmodel", MODEL),
    ("unknown1", TEXTURE),
    ("unknown4", &[Config(Type::Enum)]),
    ("unknown5", &[Config(Type::Enum)]),
    ("wearpostype", NUMBER),
];

const WMA: &[Field] = &[
    ("name", TEXT),
    ("origin", NUMBER),
    ("background", RGB),
    ("listed", FLAG),
    ("zoom", WORD),
    ("buildarea", NUMBER),
    ("subarea", NUMBER),
    ("element", &[Number, Config(Type::MapElement), Number]),
];

const WORLDMAP: &[Field] = &[
    ("format", WORD),
    ("underlays", NUMBER),
    ("flu", &[Config(Type::Flu)]),
    ("overlays", NUMBER),
    ("flo", &[Config(Type::Flo)]),
    ("square", TEXT),
    ("chunk_kind", NUMBER),
    ("map_x", NUMBER),
    ("map_z", NUMBER),
    ("chunk_x", NUMBER),
    ("chunk_z", NUMBER),
    ("floor", &[Floor]),
    ("underlay", &[Config(Type::Flu)]),
    ("overlay", &[Config(Type::Flo)]),
    ("shape_rotation", NUMBER),
    ("locs", NUMBER),
    ("loc", &[Config(Type::Loc)]),
    ("loc_shape_rotation", NUMBER),
];

const IF3: &[Field] = &[
    ("type", WORD),
    ("name", TEXT),
    ("clientcode", NUMBER),
    ("x", NUMBER),
    ("y", NUMBER),
    ("width", NUMBER),
    ("height", NUMBER),
    ("widthmode", WORD),
    ("heightmode", WORD),
    ("xmode", WORD),
    ("ymode", WORD),
    ("layer", &[Component]),
    ("hide", FLAG),
    ("scrollwidth", NUMBER),
    ("scrollheight", NUMBER),
    ("noclickthrough", FLAG),
    ("graphic", GRAPHIC),
    ("2dangle", NUMBER),
    ("tiling", FLAG),
    ("alpha", FLAG),
    ("trans", NUMBER),
    ("outline", NUMBER),
    ("graphicshadow", RGB),
    ("vflip", FLAG),
    ("hflip", FLAG),
    ("colour", RGB),
    ("fill", FLAG),
    ("model", MODEL),
    ("modelorigin", NUMBER),
    ("modelangle", NUMBER),
    ("modelzoom", NUMBER),
    ("modeloffset", NUMBER),
    ("modelanim", SEQ),
    ("modelorthog", FLAG),
    ("modelnodepth", FLAG),
    ("modelobjwidth", NUMBER),
    ("modelobjheight", NUMBER),
    ("textfont", &[Config(Type::FontMetrics)]),
    ("text", TEXT),
    ("textlineheight", NUMBER),
    ("textalignh", WORD),
    ("textalignv", WORD),
    ("textshadow", FLAG),
    ("linewid", NUMBER),
    ("linedirection", FLAG),
    ("pausebutton", FLAG),
    ("transmitop", NUMBER),
    ("targetmask", WORD),
    ("dragdepth", NUMBER),
    ("candrop", FLAG),
    ("targetable", FLAG),
    ("opbase", TEXT),
    ("pausetext", TEXT),
    ("op", TEXT),
    ("opkey", NUMBER),
    ("opcursor", CURSOR_ONLY),
    ("dragdeadzone", NUMBER),
    ("dragdeadtime", NUMBER),
    ("dragrenderbehaviour", NUMBER),
    ("targetverb", TEXT),
    ("targetobj", OBJ),
    ("targetcursor", CURSOR_ONLY),
    ("onload", HOOK),
    ("onmouseover", HOOK),
    ("onmouseleave", HOOK),
    ("ontargetleave", HOOK),
    ("ontargetenter", HOOK),
    ("onvartransmit", HOOK),
    ("oninvtransmit", HOOK),
    ("onstattransmit", HOOK),
    ("ontimer", HOOK),
    ("onop", HOOK),
    ("onmouserepeat", HOOK),
    ("onclick", HOOK),
    ("onclickrepeat", HOOK),
    ("onrelease", HOOK),
    ("onhold", HOOK),
    ("ondrag", HOOK),
    ("ondragcomplete", HOOK),
    ("onscrollwheel", HOOK),
    ("onvarctransmit", HOOK),
    ("onvarcstrtransmit", HOOK),
];

const VARBIT_FIELDS: &[Field] = &[("basevar", VARP), ("startbit", NUMBER), ("endbit", NUMBER)];

const VARCONBIT_FIELDS: &[Field] = &[
    ("basevar", &[Config(Type::VarCon)]),
    ("startbit", NUMBER),
    ("endbit", NUMBER),
];

const CONTROLLER: &[Field] = &[
    ("varcon", &[Config(Type::VarCon), Text]),
    ("varconbit", &[Config(Type::VarConBit), Number]),
];

const VAROBJ_FIELDS: &[Field] = &[("startbit", NUMBER), ("endbit", NUMBER)];

const VARC: &[Field] = &[("type", TYPE), ("scope", WORD)];

const VARCLANSETTING: &[Field] = &[
    ("type", TYPE),
    ("basevar", &[Config(Type::VarClanSetting)]),
    ("startbit", NUMBER),
    ("endbit", NUMBER),
];

const VAR_FIELDS: &[Field] = &[
    ("type", TYPE),
    ("scope", WORD),
    ("protect", FLAG),
    ("transmit", FLAG),
    ("clientcode", NUMBER),
];

pub(super) fn keys(kind: &str) -> &'static [Field] {
    match kind {
        "area" => AREA,
        "bas" => BAS,
        "billboard" => BILLBOARD,
        "defaults" => DEFAULTS,
        "if3" => IF3,
        "particleeffector" => PARTICLEEFFECTOR,
        "particleemitter" => PARTICLEEMITTER,
        "quickchatcat" => QUICKCHATCAT,
        "quickchatphrase" => QUICKCHATPHRASE,
        "wma" => WMA,
        "fm" | "fms" => WORLDMAP,
        "cursor" => CURSOR,
        "dbrow" => DBROW,
        "dbtable" => DBTABLE,
        "enum" => ENUM,
        "flo" => FLO,
        "flu" => FLU,
        "hitmark" => HITMARK,
        "hunt" => HUNT,
        "idk" => IDK,
        "inv" => INV,
        "light" => LIGHT,
        "loc" => LOC,
        "mel" => MEL,
        "mesanim" => MESANIM,
        "msi" => MSI,
        "npc" => NPC_FIELDS,
        "obj" => OBJ_FIELDS,
        "param" => PARAM_FIELDS,
        "quest" => QUEST_FIELDS,
        "seq" => SEQ_FIELDS,
        "spotanim" => SPOTANIM,
        "struct" => STRUCT,
        "varpbit" => VARBIT_FIELDS,
        "varconbit" => VARCONBIT_FIELDS,
        "varcon" => VARC,
        "controller" => CONTROLLER,
        "varc" => VARC,
        "varclansetting" => VARCLANSETTING,
        "varobj" => VAROBJ_FIELDS,
        "varn" | "varp" | "vars" => VAR_FIELDS,
        _ => &[],
    }
}

/// A key's parts in comma order. A key numbered per entry (`op1`) shares a row,
/// and the last part repeats.
pub(super) fn field(kind: &str, key: &str) -> Option<&'static [Value]> {
    match (kind, key) {
        ("material", "averagecolour") => return Some(HSL),
        ("texture", "color") => return Some(TEXTURE_COLOR),
        ("texture", "target_color") => return Some(TEXTURE_TARGET_COLOR),
        _ => {}
    }
    if kind == "if"
        && matches!(
            key,
            "colour" | "activecolour" | "overcolour" | "activeovercolour"
        )
    {
        return Some(RGB);
    }
    let table = keys(kind);
    if table.is_empty() {
        return None;
    }
    // A tile goes by its coordinates.
    if matches!(kind, "fm" | "fms") && key.starts_with("tile") {
        return Some(TILE);
    }
    if let Some(suffix) = key.strip_prefix("recol") {
        if matches!(kind, "idk" | "loc" | "npc" | "obj" | "spotanim") {
            let (suffix, values) = if let Some(suffix) = suffix.strip_prefix("hsl") {
                (suffix, HSL)
            } else {
                (suffix, RGB15)
            };
            if let Some(index) = suffix
                .strip_suffix('s')
                .or_else(|| suffix.strip_suffix('d'))
                && !index.is_empty()
                && index.bytes().all(|ch| ch.is_ascii_digit())
            {
                return Some(values);
            }
        }
        return Some(NUMBER);
    }
    if key.starts_with("retex") {
        return Some(TEXTURE);
    }
    let base = key.trim_end_matches(|ch: char| ch.is_ascii_digit());
    table
        .iter()
        .find(|(name, _)| *name == key || *name == base)
        .map(|(_, values)| *values)
}

/// What part `part` of a value is, given the part before it and its section.
pub(super) fn slot(
    values: &[Value],
    part: usize,
    first: &str,
    section: &Section,
    types: &ConfigTypes,
) -> Option<Value> {
    let by_name = |name: Option<&str>| name.and_then(Type::from_name).map(of_type);
    match *values.get(part).or(values.last())? {
        Input => by_name(section.input),
        Output => by_name(section.output),
        Declared => by_name(section.declared),
        TextureColor(kind) => (section.declared == Some(kind)).then_some(Color(Encoding::Rgb)),
        OfParam => types.params.get(first).copied().map(of_type),
        OfColumn => {
            let column = types.columns.get(&format!("{}:{first}", section.table?))?;
            (!column.is_empty()).then(|| of_type(column[part.saturating_sub(1) % column.len()]))
        }
        Tile => Some(match (first, part) {
            ("underlay", 0) => Config(Type::Flu),
            ("overlay", 0) => Config(Type::Flo),
            // The palette index after `@`.
            ("underlay" | "overlay", _) => Number,
            _ => Tile,
        }),
        Floor => Some(Config(if section.floor == Some("overlay") {
            Type::Flo
        } else {
            Type::Flu
        })),
        value => Some(value),
    }
}

fn of_type(ty: Type) -> Value {
    match ty {
        Type::String => Text,
        Type::Boolean => Flag,
        Type::Colour => Color(Encoding::Rgb),
        Type::Int | Type::Long | Type::Char | Type::Coord => Number,
        _ => Config(ty),
    }
}

pub(super) fn describe(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| match value {
            Config(ty) => ty.name(),
            AnyVar => "varp or varpbit",
            Number => "int",
            Color(Encoding::Rgb) | TextureColor(_) => "rgb",
            Color(Encoding::Argb) => "argb",
            Color(Encoding::Rgb15) => "rgb15",
            Color(Encoding::Hsl) => "hsl",
            Color(Encoding::Hsl32) => "hsl32",
            Color(Encoding::Gray12) => "grayscale",
            Flag => "boolean",
            Word => "word",
            Text => "string",
            TypeName | Declared => "type",
            Input => "inputtype",
            Output => "outputtype",
            OfParam => "param value",
            ColumnName => "dbcolumn",
            OfColumn => "column value",
            Sound => "synth or vorbis",
            Component => "component",
            Hook => "clientscript hook",
            Tile => "world map tile",
            Floor => "flu or flo",
        })
        .collect::<Vec<_>>()
        .join(", ")
}
