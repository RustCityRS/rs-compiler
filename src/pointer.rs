use std::collections::HashMap;
use std::fmt;

/// The 22 pointer types tracked by the RuneScript pointer checker.
///
/// Each variant maps to a bit position in [`PointerSet`] and has a display
/// name matching the reference TypeScript implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PointerType {
    ActivePlayer = 0,
    ActivePlayer2 = 1,
    PActivePlayer = 2,
    PActivePlayer2 = 3,
    ActiveNpc = 4,
    ActiveNpc2 = 5,
    ActiveLoc = 6,
    ActiveLoc2 = 7,
    ActiveObj = 8,
    ActiveObj2 = 9,
    FindPlayer = 10,
    FindNpc = 11,
    FindLoc = 12,
    FindObj = 13,
    FindDb = 14,
    LastCom = 15,
    LastInt = 16,
    LastItem = 17,
    LastSlot = 18,
    LastTargetslot = 19,
    LastUseitem = 20,
    LastUseslot = 21,
}

impl PointerType {
    pub const COUNT: usize = 22;

    pub const ALL: [PointerType; 22] = [
        PointerType::ActivePlayer,
        PointerType::ActivePlayer2,
        PointerType::PActivePlayer,
        PointerType::PActivePlayer2,
        PointerType::ActiveNpc,
        PointerType::ActiveNpc2,
        PointerType::ActiveLoc,
        PointerType::ActiveLoc2,
        PointerType::ActiveObj,
        PointerType::ActiveObj2,
        PointerType::FindPlayer,
        PointerType::FindNpc,
        PointerType::FindLoc,
        PointerType::FindObj,
        PointerType::FindDb,
        PointerType::LastCom,
        PointerType::LastInt,
        PointerType::LastItem,
        PointerType::LastSlot,
        PointerType::LastTargetslot,
        PointerType::LastUseitem,
        PointerType::LastUseslot,
    ];

    /// The representation string used in diagnostics, matching the TS reference.
    pub fn representation(self) -> &'static str {
        match self {
            PointerType::ActivePlayer => "active_player",
            PointerType::ActivePlayer2 => ".active_player",
            PointerType::PActivePlayer => "p_active_player",
            PointerType::PActivePlayer2 => ".p_active_player",
            PointerType::ActiveNpc => "active_npc",
            PointerType::ActiveNpc2 => ".active_npc",
            PointerType::ActiveLoc => "active_loc",
            PointerType::ActiveLoc2 => ".active_loc",
            PointerType::ActiveObj => "active_obj",
            PointerType::ActiveObj2 => ".active_obj",
            PointerType::FindPlayer => "find_player",
            PointerType::FindNpc => "find_npc",
            PointerType::FindLoc => "find_loc",
            PointerType::FindObj => "find_obj",
            PointerType::FindDb => "find_db",
            PointerType::LastCom => "last_com",
            PointerType::LastInt => "last_int",
            PointerType::LastItem => "last_item",
            PointerType::LastSlot => "last_slot",
            PointerType::LastTargetslot => "last_targetslot",
            PointerType::LastUseitem => "last_useitem",
            PointerType::LastUseslot => "last_useslot",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Look up a pointer type by its lowercase name (e.g. "active_player",
    /// ".active_player", "p_active_player").
    pub fn from_name(name: &str) -> Option<PointerType> {
        let lower = name.to_lowercase();
        Self::ALL
            .iter()
            .find(|&&ptr| ptr.representation() == lower)
            .copied()
    }
}

impl fmt::Display for PointerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.representation())
    }
}

// ---------------------------------------------------------------------------
// PointerSet — 22-bit bitmask for efficient pointer set operations
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct PointerSet(u32);

impl PointerSet {
    pub fn new() -> Self {
        PointerSet(0)
    }

    /// A set containing all 22 pointer types.
    pub fn all() -> Self {
        PointerSet((1u32 << PointerType::COUNT) - 1)
    }

    pub fn insert(&mut self, ptr: PointerType) {
        self.0 |= 1u32 << ptr.index();
    }

    pub fn remove(&mut self, ptr: PointerType) {
        self.0 &= !(1u32 << ptr.index());
    }

    pub fn contains(self, ptr: PointerType) -> bool {
        (self.0 & (1u32 << ptr.index())) != 0
    }

    pub fn union(self, other: PointerSet) -> PointerSet {
        PointerSet(self.0 | other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> PointerSetIter {
        PointerSetIter { bits: self.0 }
    }
}

pub struct PointerSetIter {
    bits: u32,
}

impl Iterator for PointerSetIter {
    type Item = PointerType;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bits == 0 {
            return None;
        }
        let idx = self.bits.trailing_zeros() as usize;
        self.bits &= self.bits - 1; // clear lowest bit
        if idx < PointerType::COUNT {
            Some(PointerType::ALL[idx])
        } else {
            None
        }
    }
}

impl FromIterator<PointerType> for PointerSet {
    fn from_iter<T: IntoIterator<Item = PointerType>>(iter: T) -> Self {
        let mut set = PointerSet::new();
        for ptr in iter {
            set.insert(ptr);
        }
        set
    }
}

// ---------------------------------------------------------------------------
// PointerHolder — describes a command's pointer requirements
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct PointerHolder {
    pub required: PointerSet,
    pub set: PointerSet,
    pub conditional_set: bool,
    pub corrupted: PointerSet,
}

// ---------------------------------------------------------------------------
// trigger_pointers — which pointers a trigger type initializes
// ---------------------------------------------------------------------------

/// Returns the set of pointers that are initialized when a script with the
/// given trigger type begins execution. Matches `ServerTriggerType.ts`.
pub fn trigger_pointers(trigger: &str) -> PointerSet {
    use PointerType::*;

    let ptrs: &[PointerType] = match trigger {
        // proc and label get ALL pointers (they inherit from caller)
        "proc" | "label" => return PointerSet::all(),

        // debugproc gets only active_player (console command context)
        "debugproc" => &[ActivePlayer],

        // Player-NPC approach/operate triggers
        "apnpc1" | "apnpc2" | "apnpc3" | "apnpc4" | "apnpc5" => {
            &[ActivePlayer, PActivePlayer, ActiveNpc]
        }
        "apnpcu" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActiveNpc,
        ],
        "apnpct" => &[ActivePlayer, PActivePlayer, ActiveNpc],
        "opnpc1" | "opnpc2" | "opnpc3" | "opnpc4" | "opnpc5" => {
            &[ActivePlayer, PActivePlayer, ActiveNpc]
        }
        "opnpcu" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActiveNpc,
        ],
        "opnpct" => &[ActivePlayer, PActivePlayer, ActiveNpc],

        // AI NPC triggers (NPC vs NPC)
        "ai_apnpc1" | "ai_apnpc2" | "ai_apnpc3" | "ai_apnpc4" | "ai_apnpc5" => {
            &[ActiveNpc, ActiveNpc2]
        }
        "ai_opnpc1" | "ai_opnpc2" | "ai_opnpc3" | "ai_opnpc4" | "ai_opnpc5" => {
            &[ActiveNpc, ActiveNpc2]
        }

        // Player-Obj approach/operate triggers
        "apobj1" | "apobj2" | "apobj3" | "apobj4" | "apobj5" => {
            &[ActivePlayer, PActivePlayer, ActiveObj]
        }
        "apobju" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActiveObj,
        ],
        "apobjt" => &[ActivePlayer, PActivePlayer, ActiveObj],
        "opobj1" | "opobj2" | "opobj3" | "opobj4" | "opobj5" => {
            &[ActivePlayer, PActivePlayer, ActiveObj]
        }
        "opobju" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActiveObj,
        ],
        "opobjt" => &[ActivePlayer, PActivePlayer, ActiveObj],

        // AI Obj triggers (NPC vs obj)
        "ai_apobj1" | "ai_apobj2" | "ai_apobj3" | "ai_apobj4" | "ai_apobj5" => {
            &[ActiveNpc, ActiveObj]
        }
        "ai_opobj1" | "ai_opobj2" | "ai_opobj3" | "ai_opobj4" | "ai_opobj5" => {
            &[ActiveNpc, ActiveObj]
        }

        // Player-Loc approach/operate triggers
        "aploc1" | "aploc2" | "aploc3" | "aploc4" | "aploc5" => {
            &[ActivePlayer, PActivePlayer, ActiveLoc]
        }
        "aplocu" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActiveLoc,
        ],
        "aploct" => &[ActivePlayer, PActivePlayer, ActiveLoc],
        "oploc1" | "oploc2" | "oploc3" | "oploc4" | "oploc5" => {
            &[ActivePlayer, PActivePlayer, ActiveLoc]
        }
        "oplocu" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActiveLoc,
        ],
        "oploct" => &[ActivePlayer, PActivePlayer, ActiveLoc],

        // AI Loc triggers (NPC vs loc)
        "ai_aploc1" | "ai_aploc2" | "ai_aploc3" | "ai_aploc4" | "ai_aploc5" => {
            &[ActiveNpc, ActiveLoc]
        }
        "ai_oploc1" | "ai_oploc2" | "ai_oploc3" | "ai_oploc4" | "ai_oploc5" => {
            &[ActiveNpc, ActiveLoc]
        }

        // Player-Player approach/operate triggers
        "applayer1" | "applayer2" | "applayer3" | "applayer4" | "applayer5" => {
            &[ActivePlayer, PActivePlayer, ActivePlayer2]
        }
        "applayeru" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActivePlayer2,
        ],
        "applayert" => &[ActivePlayer, PActivePlayer, ActivePlayer2],
        "opplayer1" | "opplayer2" | "opplayer3" | "opplayer4" | "opplayer5" => {
            &[ActivePlayer, PActivePlayer, ActivePlayer2]
        }
        "opplayeru" => &[
            ActivePlayer,
            PActivePlayer,
            LastUseitem,
            LastUseslot,
            ActivePlayer2,
        ],
        "opplayert" => &[ActivePlayer, PActivePlayer, ActivePlayer2],

        // AI Player triggers (NPC vs player)
        "ai_applayer1" | "ai_applayer2" | "ai_applayer3" | "ai_applayer4" | "ai_applayer5" => {
            &[ActiveNpc, ActivePlayer]
        }
        "ai_opplayer1" | "ai_opplayer2" | "ai_opplayer3" | "ai_opplayer4" | "ai_opplayer5" => {
            &[ActiveNpc, ActivePlayer]
        }

        // Queue trigger
        "queue" => &[ActivePlayer, PActivePlayer],

        // AI queue triggers
        "ai_queue1" | "ai_queue2" | "ai_queue3" | "ai_queue4" | "ai_queue5" | "ai_queue6"
        | "ai_queue7" | "ai_queue8" | "ai_queue9" | "ai_queue10" | "ai_queue11" | "ai_queue12"
        | "ai_queue13" | "ai_queue14" | "ai_queue15" | "ai_queue16" | "ai_queue17"
        | "ai_queue18" | "ai_queue19" | "ai_queue20" => &[ActiveNpc, LastInt],

        // Soft timer (player context only)
        "softtimer" => &[ActivePlayer],

        // Timer (player + protected)
        "timer" => &[ActivePlayer, PActivePlayer],

        // AI timer (NPC context)
        "ai_timer" => &[ActiveNpc],

        // Held item triggers
        "opheld1" | "opheld2" | "opheld3" | "opheld4" | "opheld5" => {
            &[ActivePlayer, PActivePlayer, LastItem, LastSlot]
        }
        "opheldu" => &[
            ActivePlayer,
            PActivePlayer,
            LastItem,
            LastSlot,
            LastUseitem,
            LastUseslot,
        ],
        "opheldt" => &[ActivePlayer, PActivePlayer, LastItem, LastSlot],
        // Rev-647 extensions
        "opheld6" | "opheld7" | "opheld8" | "opheld9" | "opheld10" => {
            &[ActivePlayer, PActivePlayer, LastItem, LastSlot]
        }

        // Interface button trigger
        "if_button" => &[ActivePlayer, LastCom, LastSlot, LastItem],

        // Interface close trigger
        "if_close" => &[ActivePlayer],

        // Inventory button triggers
        "inv_button1" | "inv_button2" | "inv_button3" | "inv_button4" | "inv_button5" => {
            &[ActivePlayer, LastItem, LastSlot]
        }
        // Also accept if_button1..5 aliases
        "if_button1" | "if_button2" | "if_button3" | "if_button4" | "if_button5" => {
            &[ActivePlayer, LastItem, LastSlot]
        }
        "inv_buttond" | "if_buttond" => &[ActivePlayer, LastSlot, LastTargetslot],
        // Rev-647 extensions
        "inv_button6" | "inv_button7" | "inv_button8" | "inv_button9" | "inv_button10" => {
            &[ActivePlayer, LastItem, LastSlot]
        }

        // Walk trigger
        "walktrigger" => &[ActivePlayer, PActivePlayer],

        // Global triggers
        "login" | "logout" | "tutorial" | "switch_window_mode" => &[ActivePlayer, PActivePlayer],
        "advancestat" | "changestat" => &[ActivePlayer, PActivePlayer],
        "mapzone" | "mapzoneexit" | "zone" | "zoneexit" => &[ActivePlayer, PActivePlayer],

        // AI lifecycle triggers
        "ai_spawn" | "ai_despawn" => &[ActiveNpc],

        // AI walktrigger
        "ai_walktrigger" => &[ActiveNpc],

        // Unknown trigger — no pointers initialized
        _ => return PointerSet::new(),
    };

    ptrs.iter().copied().collect()
}

// ---------------------------------------------------------------------------
// command_pointers — pointer requirements for every engine command
// ---------------------------------------------------------------------------

/// Builds the full mapping from command name to its pointer requirements.
///
/// Both primary and secondary (`.` prefixed) variants are generated as
/// separate entries. The keys use lowercase command names matching the
/// SymbolRegistry command map.
///
/// Transcribed from `ScriptOpcodePointers.ts`.
pub fn command_pointers() -> HashMap<String, PointerHolder> {
    use PointerType::*;

    let mut map = HashMap::new();

    // Helper closures to reduce boilerplate
    let mut add = |name: &str,
                   require: &[PointerType],
                   set: &[PointerType],
                   corrupt: &[PointerType],
                   require2: &[PointerType],
                   set2: &[PointerType],
                   corrupt2: &[PointerType],
                   conditional: bool| {
        // Primary entry
        let holder = PointerHolder {
            required: require.iter().copied().collect(),
            set: set.iter().copied().collect(),
            corrupted: corrupt.iter().copied().collect(),
            conditional_set: conditional,
        };
        map.insert(name.to_lowercase(), holder);

        // Secondary entry (dot-prefixed) if any require2/set2/corrupt2 are specified
        if !require2.is_empty() || !set2.is_empty() || !corrupt2.is_empty() {
            let holder2 = PointerHolder {
                required: require2.iter().copied().collect(),
                set: set2.iter().copied().collect(),
                corrupted: corrupt2.iter().copied().collect(),
                conditional_set: conditional,
            };
            map.insert(format!(".{}", name.to_lowercase()), holder2);
        }
    };

    // The POINTER_GROUP_FIND shorthand
    let find_group: &[PointerType] = &[FindPlayer, FindNpc, FindLoc, FindObj, FindDb];
    // Corruption set for p_delay / p_arrivedelay style commands:
    // everything except active is assumed corrupted
    let delay_corrupt: Vec<PointerType> = {
        let mut v: Vec<PointerType> = find_group.to_vec();
        v.extend_from_slice(&[
            LastCom,
            LastInt,
            LastItem,
            LastSlot,
            LastTargetslot,
            LastUseitem,
            LastUseslot,
        ]);
        v
    };

    // -----------------------------------------------------------------------
    // Player ops
    // -----------------------------------------------------------------------
    add(
        "allowdesign",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "anim",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add("readyanim", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add("runanim", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add("turnanim", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "walkanim_b",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("walkanim", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "walkanim_l",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "walkanim_r",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "buildappearance",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "busy",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "busy2",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "cam_lookat",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "cam_moveto",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "cam_reset",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "cam_shake",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "clearqueue",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "clearsofttimer",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "cleartimer",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "gettimer",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "coord",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "damage",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "displayname",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "facesquare",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "finduid",
        &[],
        &[ActivePlayer],
        &[PActivePlayer],
        &[],
        &[ActivePlayer2],
        &[PActivePlayer2],
        true,
    );
    add(
        "gender",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "getqueue",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_advance",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "headicons_get",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "headicons_set",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "healenergy",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "hint_coord",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "hint_npc",
        &[ActivePlayer, ActiveNpc],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "hint_pl",
        &[ActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("hint_stop", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add("huntall", &[], &[FindPlayer], &[], &[], &[], &[], false);
    add(
        "huntnext",
        &[FindPlayer],
        &[ActivePlayer],
        &[],
        &[FindPlayer],
        &[ActivePlayer2],
        &[],
        true,
    );
    add(
        "npc_hunt",
        &[],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        false,
    );
    add("npc_huntall", &[], &[FindNpc], &[], &[], &[], &[], false);
    add(
        "npc_hasop",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "if_close",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add("tut_close", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "if_openchat",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("tut_open", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    // 2004scape (rev 225) names this command `if_openmain`. Rev 647 renamed it
    // to `if_opentop` (since `if_openmain` is reserved for opening a modal in
    // the main viewport). Both names get the same pointer rules so the compiler
    // accepts content from either revision.
    add(
        "if_openmain",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "if_opentop",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "if_openmain_side",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "if_openoverlay",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "if_openside",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_setanim",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_setcolour",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_sethide",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "if_setmodel",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_setnpchead",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_setobject",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_setplayerhead",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_setposition",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_addresumebutton",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("if_settab", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "if_settabactive",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add("tut_flash", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "if_settext",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "last_login_info",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("last_com", &[LastCom], &[], &[], &[], &[], &[], false);
    add("last_int", &[LastInt], &[], &[], &[], &[], &[], false);
    add("last_item", &[LastItem], &[], &[], &[], &[], &[], false);
    add("last_slot", &[LastSlot], &[], &[], &[], &[], &[], false);
    add(
        "last_targetslot",
        &[LastTargetslot],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "last_useitem",
        &[LastUseitem],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "last_useslot",
        &[LastUseslot],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "longqueue",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "longqueue*",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add("lowmem", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "mes",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "midi_jingle",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("midi_song", &[ActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "minimap_toggle",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "name",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "p_aprange",
        &[PActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_arrivedelay",
        &[PActivePlayer],
        &[],
        &delay_corrupt,
        &[],
        &[],
        &[],
        false,
    );
    {
        // p_countdialog: sets last_int, corrupts everything except active + last_int
        let mut corrupt = delay_corrupt.clone();
        corrupt.retain(|p| *p != LastInt);
        add(
            "p_countdialog",
            &[PActivePlayer],
            &[LastInt],
            &corrupt,
            &[],
            &[],
            &[],
            false,
        );
    }
    add(
        "p_delay",
        &[PActivePlayer],
        &[],
        &delay_corrupt,
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_exactmove",
        &[PActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_finduid",
        &[],
        &[PActivePlayer, ActivePlayer],
        &[],
        &[],
        &[PActivePlayer2, ActivePlayer2],
        &[],
        true,
    );
    add(
        "p_locmerge",
        &[PActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add("p_logout", &[PActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "p_preventlogout",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add("p_opheld", &[PActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "p_oploc",
        &[PActivePlayer, ActiveLoc],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_opnpc",
        &[PActivePlayer, ActiveNpc],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_opnpct",
        &[PActivePlayer, ActiveNpc],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_opobj",
        &[PActivePlayer, ActiveObj],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_opplayer",
        &[PActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[PActivePlayer2, ActivePlayer],
        &[],
        &[],
        false,
    );
    add(
        "p_opplayert",
        &[PActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[PActivePlayer2, ActivePlayer],
        &[],
        &[],
        false,
    );
    {
        // p_pausebutton: sets last_com, corrupts everything except active + last_com
        let mut corrupt = delay_corrupt.clone();
        corrupt.retain(|p| *p != LastCom);
        add(
            "p_pausebutton",
            &[PActivePlayer],
            &[LastCom],
            &corrupt,
            &[],
            &[],
            &[],
            false,
        );
    }
    add(
        "p_stopaction",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "p_clearpendingaction",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "p_telejump",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "p_teleport",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "p_walk",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "projanim_pl",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "queue",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "queue*",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "say",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add("setidkit", &[PActivePlayer], &[], &[], &[], &[], &[], false);
    add(
        "walktrigger",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "getwalktrigger",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "settimer",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "softtimer",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "sound_synth",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "spotanim_pl",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "staffmodlevel",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_add",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_base",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_heal",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_sub",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_boost",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_drain",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "stat_random",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "uid",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "weakqueue",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "weakqueue*",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "findhero",
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        &[ActivePlayer],
        &[],
        true,
    );
    add(
        "both_heropoints",
        &[ActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[ActivePlayer2, ActivePlayer],
        &[],
        &[],
        false,
    );
    add(
        "setgender",
        &[PActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "setidkcolour",
        &[PActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "p_animprotect",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "runenergy",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "weight",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "p_run",
        &[PActivePlayer],
        &[],
        &[],
        &[PActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "if_setscrollpos",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "if_movesub",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "set_player_op",
        &[ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "set_skill_level",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "strongqueue",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "strongqueue*",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );

    // -----------------------------------------------------------------------
    // NPC ops
    // -----------------------------------------------------------------------
    add(
        "npc_add",
        &[],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        false,
    );
    add(
        "npc_anim",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_basestat",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_category",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_changetype",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_changetype_keepall",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_coord",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_damage",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_del",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    {
        // npc_delay: corrupts p_active_player, p_active_player2, find_*, last_*
        let mut corrupt: Vec<PointerType> = vec![PActivePlayer, PActivePlayer2];
        corrupt.extend_from_slice(find_group);
        corrupt.extend_from_slice(&[
            LastCom,
            LastInt,
            LastItem,
            LastSlot,
            LastTargetslot,
            LastUseitem,
            LastUseslot,
        ]);
        add(
            "npc_delay",
            &[ActiveNpc],
            &[],
            &corrupt,
            &[ActiveNpc2],
            &[],
            &[],
            false,
        );
    }
    add(
        "npc_facesquare",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_find",
        &[],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        true,
    );
    add(
        "npc_findcat",
        &[],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        true,
    );
    add("npc_findallany", &[], &[FindNpc], &[], &[], &[], &[], false);
    add("npc_findall", &[], &[FindNpc], &[], &[], &[], &[], false);
    add(
        "npc_findallzone",
        &[],
        &[FindNpc],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "npc_findnext",
        &[FindNpc],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        true,
    );
    add(
        "npc_findexact",
        &[],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        true,
    );
    add(
        "npc_findhero",
        &[ActiveNpc],
        &[ActivePlayer],
        &[],
        &[ActiveNpc2],
        &[ActivePlayer2],
        &[],
        true,
    );
    add(
        "npc_finduid",
        &[],
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        true,
    );
    add(
        "npc_getmode",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_heropoints",
        &[ActiveNpc, ActivePlayer],
        &[],
        &[],
        &[],
        &[],
        &[],
        false,
    );
    add(
        "npc_name",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_param",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_queue",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_range",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_say",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_sethunt",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_sethuntmode",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_setmode",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_walktrigger",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_settimer",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_stat",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_statadd",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_statheal",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_statsub",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_tele",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_type",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_uid",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "projanim_npc",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "spotanim_npc",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_walk",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    add(
        "npc_attackrange",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );
    {
        // npc_arrivedelay: same corruption set as npc_delay
        let mut corrupt: Vec<PointerType> = vec![PActivePlayer, PActivePlayer2];
        corrupt.extend_from_slice(find_group);
        corrupt.extend_from_slice(&[
            LastCom,
            LastInt,
            LastItem,
            LastSlot,
            LastTargetslot,
            LastUseitem,
            LastUseslot,
        ]);
        add(
            "npc_arrivedelay",
            &[ActiveNpc],
            &[],
            &corrupt,
            &[ActiveNpc2],
            &[],
            &[],
            false,
        );
    }
    add(
        "npc_inrange",
        &[ActiveNpc],
        &[],
        &[],
        &[ActiveNpc2],
        &[],
        &[],
        false,
    );

    // -----------------------------------------------------------------------
    // Loc ops
    // -----------------------------------------------------------------------
    add(
        "loc_add",
        &[],
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        false,
    );
    add(
        "loc_angle",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_anim",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_category",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_change",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_coord",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_del",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_find",
        &[],
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        true,
    );
    add(
        "loc_findallzone",
        &[],
        &[FindLoc],
        &[],
        &[],
        &[FindLoc],
        &[],
        false,
    );
    add(
        "loc_findnext",
        &[FindLoc],
        &[ActiveLoc],
        &[],
        &[FindLoc],
        &[ActiveLoc2],
        &[],
        true,
    );
    add(
        "loc_name",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_param",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_shape",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );
    add(
        "loc_type",
        &[ActiveLoc],
        &[],
        &[],
        &[ActiveLoc2],
        &[],
        &[],
        false,
    );

    // -----------------------------------------------------------------------
    // Obj ops
    // -----------------------------------------------------------------------
    add(
        "obj_add",
        &[ActivePlayer],
        &[ActiveObj],
        &[],
        &[ActivePlayer2],
        &[ActiveObj2],
        &[],
        false,
    );
    add(
        "obj_addall",
        &[],
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        false,
    );
    add(
        "obj_coord",
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        &[],
        false,
    );
    add(
        "obj_count",
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        &[],
        false,
    );
    add(
        "obj_del",
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        &[],
        false,
    );
    add(
        "obj_name",
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        &[],
        false,
    );
    add(
        "obj_param",
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        &[],
        false,
    );
    add(
        "obj_takeitem",
        &[ActiveObj, ActivePlayer],
        &[],
        &[],
        &[ActiveObj2, ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "obj_type",
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        &[],
        false,
    );
    add(
        "obj_find",
        &[],
        &[ActiveObj],
        &[],
        &[],
        &[ActiveObj2],
        &[],
        false,
    );
    add(
        "obj_findallzone",
        &[],
        &[FindObj],
        &[],
        &[],
        &[FindObj],
        &[],
        false,
    );
    add(
        "obj_findnext",
        &[FindObj],
        &[ActiveObj],
        &[],
        &[FindObj],
        &[ActiveObj2],
        &[],
        true,
    );

    // -----------------------------------------------------------------------
    // Inventory ops
    // -----------------------------------------------------------------------
    add(
        "inv_add",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_changeslot",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_clear",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_del",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_delslot",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_dropitem",
        &[ActivePlayer],
        &[ActiveObj],
        &[],
        &[ActivePlayer2],
        &[ActiveObj2],
        &[],
        false,
    );
    add(
        "inv_dropitem_delayed",
        &[ActivePlayer],
        &[ActiveObj],
        &[],
        &[ActivePlayer2],
        &[ActiveObj2],
        &[],
        false,
    );
    add(
        "inv_dropslot",
        &[ActivePlayer],
        &[ActiveObj],
        &[],
        &[ActivePlayer2],
        &[ActiveObj2],
        &[],
        false,
    );
    add(
        "inv_freespace",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_getnum",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_getobj",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_itemspace",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_itemspace2",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_movefromslot",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_movetoslot",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "both_moveinv",
        &[ActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[ActivePlayer2, ActivePlayer],
        &[],
        &[],
        false,
    );
    add(
        "inv_moveitem",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_moveitem_cert",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_moveitem_uncert",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_setslot",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_total",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_totalcat",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_transmit",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "invother_transmit",
        &[ActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[ActivePlayer2, ActivePlayer],
        &[],
        &[],
        false,
    );
    add(
        "inv_stoptransmit",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "both_dropslot",
        &[ActivePlayer, ActivePlayer2],
        &[],
        &[],
        &[ActivePlayer2, ActivePlayer],
        &[],
        &[],
        false,
    );
    add(
        "inv_dropall",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_totalparam",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );
    add(
        "inv_totalparam_stack",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );

    // -----------------------------------------------------------------------
    // String ops
    // -----------------------------------------------------------------------
    add(
        "text_gender",
        &[ActivePlayer],
        &[],
        &[],
        &[ActivePlayer2],
        &[],
        &[],
        false,
    );

    // -----------------------------------------------------------------------
    // DB ops
    // -----------------------------------------------------------------------
    add("db_findnext", &[FindDb], &[], &[], &[], &[], &[], false);
    add("db_find", &[], &[FindDb], &[], &[], &[], &[], false);
    add("db_find_refine", &[FindDb], &[], &[], &[], &[], &[], false);
    add("db_listall", &[], &[FindDb], &[], &[], &[], &[], false);
    add(
        "db_listall_with_count",
        &[],
        &[FindDb],
        &[],
        &[],
        &[],
        &[],
        false,
    );

    map
}
