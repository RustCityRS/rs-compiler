//! The opcode of every instruction and engine command, the same at every revision.
//! A revision's `command.pack` lists the commands it has, each at its opcode here.

macro_rules! opcodes {
    ($($constant:ident = $opcode:literal => $command:literal $(| $alias:literal)*;)*) => {
        $(pub const $constant: u16 = $opcode;)*

        /// Every name a `command.pack` may give a command, with its opcode.
        pub const COMMANDS: &[(&str, u16)] = &[$(($command, $opcode), $(($alias, $opcode),)*)*];

        #[cfg(test)]
        const CONSTANTS: &[(&str, &str)] = &[$((stringify!($constant), $command),)*];
    };
}

opcodes! {
    // Instructions the compiler emits
    PUSH_CONSTANT_INT = 0 => "push_constant_int";
    PUSH_VARP = 1 => "push_varp";
    POP_VARP = 2 => "pop_varp";
    PUSH_CONSTANT_STRING = 3 => "push_constant_string";
    PUSH_VARN = 4 => "push_varn";
    POP_VARN = 5 => "pop_varn";
    BRANCH = 6 => "branch";
    BRANCH_NOT = 7 => "branch_not";
    BRANCH_EQUALS = 8 => "branch_equals";
    BRANCH_LESS_THAN = 9 => "branch_less_than";
    BRANCH_GREATER_THAN = 10 => "branch_greater_than";
    PUSH_VARS = 11 => "push_vars";
    POP_VARS = 12 => "pop_vars";
    RETURN = 21 => "return";
    GOSUB = 22 => "gosub";
    JUMP = 23 => "jump";
    SWITCH = 24 => "switch";
    PUSH_VARBIT = 25 => "push_varbit";
    POP_VARBIT = 27 => "pop_varbit";
    BRANCH_LESS_THAN_OR_EQUALS = 31 => "branch_less_than_or_equals";
    BRANCH_GREATER_THAN_OR_EQUALS = 32 => "branch_greater_than_or_equals";
    PUSH_INT_LOCAL = 33 => "push_int_local";
    POP_INT_LOCAL = 34 => "pop_int_local";
    PUSH_STRING_LOCAL = 35 => "push_string_local";
    POP_STRING_LOCAL = 36 => "pop_string_local";
    JOIN_STRING = 37 => "join_string";
    POP_INT_DISCARD = 38 => "pop_int_discard";
    POP_STRING_DISCARD = 39 => "pop_string_discard";
    GOSUB_WITH_PARAMS = 40 => "gosub_with_params";
    JUMP_WITH_PARAMS = 41 => "jump_with_params";
    PUSH_VARC_INT = 42 => "push_varc_int";
    POP_VARC_INT = 43 => "pop_varc_int";
    DEFINE_ARRAY = 44 => "define_array";
    PUSH_ARRAY_INT = 45 => "push_array_int";
    POP_ARRAY_INT = 46 => "pop_array_int";
    PUSH_VARC_STRING = 47 => "push_varc_string";
    POP_VARC_STRING = 48 => "pop_varc_string";
    PUSH_CONSTANT_LONG = 54 => "push_constant_long";
    PUSH_LONG_LOCAL = 55 => "push_long_local";
    POP_LONG_LOCAL = 56 => "pop_long_local";
    POP_LONG_DISCARD = 57 => "pop_long_discard";
    LONG_BRANCH_NOT = 68 => "long_branch_not";
    LONG_BRANCH_EQUALS = 69 => "long_branch_equals";
    LONG_BRANCH_LESS_THAN = 70 => "long_branch_less_than";
    LONG_BRANCH_GREATER_THAN = 71 => "long_branch_greater_than";
    LONG_BRANCH_LESS_THAN_OR_EQUALS = 72 => "long_branch_less_than_or_equals";
    LONG_BRANCH_GREATER_THAN_OR_EQUALS = 73 => "long_branch_greater_than_or_equals";
    PUSH_VARCLAN = 74 => "push_varclan";
    POP_VARCLAN = 75 => "pop_varclan";
    PUSH_VAROBJ = 76 => "push_varobj";
    POP_VAROBJ = 77 => "pop_varobj";
    PUSH_VARNBIT = 78 => "push_varnbit";
    POP_VARNBIT = 79 => "pop_varnbit";
    PUSH_VARCON = 80 => "push_varcon";
    POP_VARCON = 81 => "pop_varcon";
    POP_CLANSETTING_INT = 82 => "pop_clansetting_int";
    POP_CLANSETTING_VARBIT = 83 => "pop_clansetting_varbit";
    POP_CLANSETTING_LONG = 84 => "pop_clansetting_long";
    POP_CLANSETTING_STRING = 85 => "pop_clansetting_string";
    OBJ_BRANCH_EQUALS = 86 => "obj_branch_equals";
    OBJ_BRANCH_NOT = 87 => "obj_branch_not";
    PUSH_VARCONBIT = 88 => "push_varconbit";
    POP_VARCONBIT = 89 => "pop_varconbit";
    PUSH_CLANSETTING_INT = 112 => "push_clansetting_int";
    PUSH_CLANSETTING_VARBIT = 113 => "push_clansetting_varbit";
    PUSH_CLANSETTING_LONG = 114 => "push_clansetting_long";
    PUSH_CLANSETTING_STRING = 115 => "push_clansetting_string";

    // Server
    COORDX = 1000 => "coordx";
    COORDY = 1001 => "coordy";
    COORDZ = 1002 => "coordz";
    DISTANCE = 1003 => "distance";
    INZONE = 1004 => "inzone";
    LINEOFSIGHT = 1005 => "lineofsight";
    LINEOFWALK = 1006 => "lineofwalk";
    MAP_BLOCKED = 1007 => "map_blocked";
    MAP_CLOCK = 1008 => "map_clock";
    MAP_FINDSQUARE = 1009 => "map_findsquare";
    MAP_INDOORS = 1010 => "map_indoors";
    MAP_LIVE = 1011 => "map_live";
    MAP_LOCADDUNSAFE = 1012 => "map_locaddunsafe";
    MAP_MEMBERS = 1013 => "map_members";
    MAP_MULTIWAY = 1014 => "map_multiway";
    MAP_PLAYERCOUNT = 1015 => "map_playercount";
    MOVECOORD = 1016 => "movecoord";
    PLAYERCOUNT = 1017 => "playercount";
    PROJANIM_MAP = 1018 => "projanim_map";
    SEQLENGTH = 1019 => "seqlength";
    SPOTANIM_MAP = 1020 => "spotanim_map";
    WORLD_DELAY = 1021 => "world_delay";
    MIDI_LENGTH = 1022 => "midi_length";
    MAP_LOC = 1023 => "map_loc";
    SOUND_AREA = 1024 => "sound_area";
    INAREA = 1025 => "inarea";
    PROJANIM_HALFSQ = 1026 => "projanim_halfsq";
    SOUND_VORBIS_AREA = 1027 => "sound_vorbis_area";
    TEXT_COORD = 1028 => "text_coord";

    // Player
    AFK_EVENT = 2000 => "afk_event";
    ALLOWDESIGN = 2001 => "allowdesign";
    ANIM = 2002 => "anim";
    BOTH_HEROPOINTS = 2003 => "both_heropoints";
    BUILDAPPEARANCE = 2004 => "buildappearance";
    BUSY = 2005 => "busy";
    BUSY2 = 2006 => "busy2";
    CAM_LOOKAT = 2007 => "cam_lookat";
    CAM_MOVETO = 2008 => "cam_moveto";
    CAM_RESET = 2009 => "cam_reset";
    CAM_SHAKE = 2010 => "cam_shake";
    CLEARQUEUE = 2011 => "clearqueue";
    CLEARSOFTTIMER = 2012 => "clearsofttimer";
    CLEARTIMER = 2013 => "cleartimer";
    COORD = 2014 => "coord";
    DAMAGE = 2015 => "damage";
    DISPLAYNAME = 2016 => "displayname";
    FACESQUARE = 2017 => "facesquare";
    FINDHERO = 2018 => "findhero";
    FINDUID = 2019 => "finduid";
    GENDER = 2020 => "gender";
    GETQUEUE = 2021 => "getqueue";
    GETTIMER = 2022 => "gettimer";
    GETWALKTRIGGER = 2023 => "getwalktrigger";
    HEADICONS_GET = 2024 => "headicons_get";
    HEADICONS_SET = 2025 => "headicons_set";
    HEALENERGY = 2026 => "healenergy";
    HINT_COORD = 2027 => "hint_coord";
    HINT_NPC = 2028 => "hint_npc";
    HINT_PL = 2029 => "hint_pl";
    HINT_STOP = 2030 => "hint_stop";
    HUNTALL = 2031 => "huntall";
    HUNTNEXT = 2032 => "huntnext";
    IF_CLOSE = 2033 => "if_close";
    IF_OPENCHAT = 2034 => "if_openchat";
    IF_OPENMAIN_SIDE = 2035 => "if_openmain_side";
    IF_OPENMAIN = 2036 => "if_openmain";
    IF_OPENSIDE = 2037 => "if_openside";
    IF_SETANIM = 2038 => "if_setanim";
    IF_SETCOLOUR = 2039 => "if_setcolour";
    IF_SETHIDE = 2040 => "if_sethide";
    IF_SETMODEL = 2041 => "if_setmodel";
    IF_SETNPCHEAD = 2042 => "if_setnpchead";
    IF_SETOBJECT = 2043 => "if_setobject";
    IF_SETPLAYERHEAD = 2044 => "if_setplayerhead";
    IF_SETPOSITION = 2045 => "if_setposition";
    IF_SETRECOL = 2046 => "if_setrecol";
    IF_SETRESUMEBUTTONS = 2047 => "if_setresumebuttons";
    IF_SETTAB = 2048 => "if_settab";
    IF_SETTABACTIVE = 2049 => "if_settabactive";
    IF_SETTEXT = 2050 => "if_settext";
    LAST_COM = 2051 => "last_com";
    LAST_INT = 2052 => "last_int";
    LAST_ITEM = 2053 => "last_item";
    LAST_LOGIN_INFO = 2054 => "last_login_info";
    LAST_SLOT = 2055 => "last_slot";
    LAST_TARGETSLOT = 2056 => "last_targetslot";
    LAST_USEITEM = 2057 => "last_useitem";
    LAST_USESLOT = 2058 => "last_useslot";
    LONGQUEUE = 2059 => "longqueue";
    LONGQUEUEVARARG = 2060 => "longqueuevararg" | "longqueue*";
    LOWMEM = 2061 => "lowmem";
    MES = 2062 => "mes";
    MIDI_JINGLE = 2063 => "midi_jingle";
    MIDI_SONG = 2064 => "midi_song";
    NAME = 2065 => "name";
    P_ANIMPROTECT = 2066 => "p_animprotect";
    P_APRANGE = 2067 => "p_aprange";
    P_ARRIVEDELAY = 2068 => "p_arrivedelay";
    P_CLEARPENDINGACTION = 2069 => "p_clearpendingaction";
    P_COUNTDIALOG = 2070 => "p_countdialog";
    P_DELAY = 2071 => "p_delay";
    P_EXACTMOVE = 2072 => "p_exactmove";
    P_FINDUID = 2073 => "p_finduid";
    P_LOCMERGE = 2074 => "p_locmerge";
    P_LOGOUT = 2075 => "p_logout";
    P_OPHELD = 2076 => "p_opheld";
    P_OPLOC = 2077 => "p_oploc";
    P_OPNPC = 2078 => "p_opnpc";
    P_OPNPCT = 2079 => "p_opnpct";
    P_OPOBJ = 2080 => "p_opobj";
    P_OPPLAYER = 2081 => "p_opplayer";
    P_OPPLAYERT = 2082 => "p_opplayert";
    P_PAUSEBUTTON = 2083 => "p_pausebutton";
    P_PREVENTLOGOUT = 2084 => "p_preventlogout";
    P_RUN = 2085 => "p_run";
    P_STOPACTION = 2086 => "p_stopaction";
    P_TELEJUMP = 2087 => "p_telejump";
    P_TELEPORT = 2088 => "p_teleport";
    P_WALK = 2089 => "p_walk";
    PLAYERMEMBER = 2090 => "playermember";
    PROJANIM_PL = 2091 => "projanim_pl";
    QUEUE = 2092 => "queue";
    QUEUEVARARG = 2093 => "queuevararg" | "queue*";
    READYANIM = 2094 => "readyanim";
    RUNANIM = 2095 => "runanim";
    RUNENERGY = 2096 => "runenergy";
    SAY = 2097 => "say";
    SESSION_LOG = 2098 => "session_log";
    SETGENDER = 2099 => "setgender";
    SETIDKIT = 2100 => "setidkit";
    SETSKINCOLOUR = 2101 => "setskincolour";
    SETTIMER = 2102 => "settimer";
    SOFTTIMER = 2103 => "softtimer";
    SOUND_SYNTH = 2104 => "sound_synth";
    SPOTANIM_PL = 2105 => "spotanim_pl";
    STAFFMODLEVEL = 2106 => "staffmodlevel";
    STAT_ADD = 2107 => "stat_add";
    STAT_ADVANCE = 2108 => "stat_advance";
    STAT_BASE = 2109 => "stat_base";
    STAT_BOOST = 2110 => "stat_boost";
    STAT_DRAIN = 2111 => "stat_drain";
    STAT_HEAL = 2112 => "stat_heal";
    STAT_RANDOM = 2113 => "stat_random";
    STAT_SUB = 2114 => "stat_sub";
    STAT_TOTAL = 2115 => "stat_total";
    STAT = 2116 => "stat";
    STRONGQUEUE = 2117 => "strongqueue";
    STRONGQUEUEVARARG = 2118 => "strongqueuevararg" | "strongqueue*";
    TURNANIM = 2119 => "turnanim";
    TUT_CLOSE = 2120 => "tut_close";
    TUT_FLASH = 2121 => "tut_flash";
    TUT_OPEN = 2122 => "tut_open";
    UID = 2123 => "uid";
    WALKANIM_B = 2124 => "walkanim_b";
    WALKANIM_L = 2125 => "walkanim_l";
    WALKANIM_R = 2126 => "walkanim_r";
    WALKANIM = 2127 => "walkanim";
    WALKTRIGGER = 2128 => "walktrigger";
    WEAKQUEUE = 2129 => "weakqueue";
    WEAKQUEUEVARARG = 2130 => "weakqueuevararg" | "weakqueue*";
    WEALTH_EVENT = 2131 => "wealth_event";
    WEIGHT = 2132 => "weight";
    SETIDKCOLOUR = 2133 => "setidkcolour";
    BUFFER_FULL = 2134 => "buffer_full";
    LOWMEMORY = 2135 => "lowmemory";
    HINT_PLAYER = 2136 => "hint_player";
    BAS_RUNNING = 2137 => "bas_running";
    BAS_READYANIM = 2138 => "bas_readyanim";
    BAS_WALK_F = 2139 => "bas_walk_f";
    BAS_WALK_B = 2140 => "bas_walk_b";
    BAS_WALK_L = 2141 => "bas_walk_l";
    BAS_WALK_R = 2142 => "bas_walk_r";
    BAS_TURNONSPOT = 2143 => "bas_turnonspot";
    IF_SETSCROLLPOS = 2144 => "if_setscrollpos";
    IF_OPENOVERLAY = 2145 => "if_openoverlay";
    PLAYER_FINDALLZONE = 2146 => "player_findallzone";
    PLAYER_FINDNEXT = 2147 => "player_findnext";
    SET_PLAYER_OP = 2148 => "set_player_op";
    IF_ADDRESUMEBUTTON = 2149 => "if_addresumebutton";
    MINIMAP_TOGGLE = 2150 => "minimap_toggle";
    SET_SKILL_LEVEL = 2151 => "set_skill_level";
    P_TRANSMOGRIFY = 2152 => "p_transmogrify";
    P_TEMPRUN = 2153 => "p_temprun";
    P_PROJANIM_HALFSQ = 2154 => "p_projanim_halfsq";
    P_LOADDELAY = 2155 => "p_loaddelay";
    APPLYDESIGN = 2156 => "applydesign";
    GETWINDOWMODE = 2157 => "getwindowmode";
    HINT_COORD_SLOT = 2158 => "hint_coord_slot";
    HINT_NPC_SLOT = 2159 => "hint_npc_slot";
    HINT_STOP_SLOT = 2160 => "hint_stop_slot";
    IF_CLOSESUB = 2161 => "if_closesub";
    IF_MOVESUB = 2162 => "if_movesub";
    IF_OPENMAINOVERLAY = 2163 => "if_openmainoverlay";
    IF_OPENSUB = 2164 => "if_opensub";
    IF_OPENTOP = 2165 => "if_opentop";
    IF_OPENTOP_MODE = 2166 => "if_opentop_mode";
    IF_SETEVENTS = 2167 => "if_setevents";
    IF_SETGRAPHIC = 2168 => "if_setgraphic";
    LAST_COORD = 2169 => "last_coord";
    LAST_STRING = 2170 => "last_string";
    LAST_TARGETCOM = 2171 => "last_targetcom";
    MES_TYPED = 2172 => "mes_typed";
    MUSIC_PLAYJINGLE = 2173 => "music_playjingle";
    MUSIC_PLAYSONG = 2174 => "music_playsong";
    P_CLANFORUMQFCDIALOG = 2175 => "p_clanforumqfcdialog";
    P_GET_SUGGESTION = 2176 => "p_get_suggestion";
    P_HSLDIALOG = 2177 => "p_hsldialog";
    P_NAMEDIALOG = 2178 => "p_namedialog";
    P_NEXT_SUGGESTIONS = 2179 => "p_next_suggestions";
    P_OBJDIALOG = 2180 => "p_objdialog";
    P_PREV_SUGGESTIONS = 2181 => "p_prev_suggestions";
    P_REQUEST_MORE_SUGGESTIONS = 2182 => "p_request_more_suggestions";
    P_REST = 2183 => "p_rest";
    P_SETDISPLAYNAME = 2184 => "p_setdisplayname";
    P_STRINGDIALOG = 2185 => "p_stringdialog";
    RUNCLIENTSCRIPT = 2186 => "runclientscript";
    RUNCLIENTSCRIPTVARARG = 2187 => "runclientscriptvararg";
    SOUND_SPEECH_STOP = 2188 => "sound_speech_stop";
    SOUND_SPEECH_VOLUME = 2189 => "sound_speech_volume";

    // Npc
    NPC_ADD = 2500 => "npc_add";
    NPC_ANIM = 2501 => "npc_anim";
    NPC_ARRIVEDELAY = 2502 => "npc_arrivedelay";
    NPC_ATTACKRANGE = 2503 => "npc_attackrange";
    NPC_BASESTAT = 2504 => "npc_basestat";
    NPC_CATEGORY = 2505 => "npc_category";
    NPC_CHANGETYPE_KEEPALL = 2506 => "npc_changetype_keepall";
    NPC_CHANGETYPE = 2507 => "npc_changetype";
    NPC_COORD = 2508 => "npc_coord";
    NPC_DAMAGE = 2509 => "npc_damage";
    NPC_DEL = 2510 => "npc_del";
    NPC_DELAY = 2511 => "npc_delay";
    NPC_FACESQUARE = 2512 => "npc_facesquare";
    NPC_FIND = 2513 => "npc_find";
    NPC_FINDALL = 2514 => "npc_findall";
    NPC_FINDALLANY = 2515 => "npc_findallany";
    NPC_FINDALLZONE = 2516 => "npc_findallzone";
    NPC_FINDCAT = 2517 => "npc_findcat";
    NPC_FINDEXACT = 2518 => "npc_findexact";
    NPC_FINDHERO = 2519 => "npc_findhero";
    NPC_FINDNEXT = 2520 => "npc_findnext";
    NPC_FINDUID = 2521 => "npc_finduid";
    NPC_GETMODE = 2522 => "npc_getmode";
    NPC_HASOP = 2523 => "npc_hasop";
    NPC_HEROPOINTS = 2524 => "npc_heropoints";
    NPC_HUNT = 2525 => "npc_hunt";
    NPC_HUNTALL = 2526 => "npc_huntall";
    NPC_INRANGE = 2527 => "npc_inrange";
    NPC_NAME = 2528 => "npc_name";
    NPC_PARAM = 2529 => "npc_param";
    NPC_QUEUE = 2530 => "npc_queue";
    NPC_RANGE = 2531 => "npc_range";
    NPC_SAY = 2532 => "npc_say";
    NPC_SETHUNT = 2533 => "npc_sethunt";
    NPC_SETHUNTMODE = 2534 => "npc_sethuntmode";
    NPC_SETMODE = 2535 => "npc_setmode";
    NPC_SETTIMER = 2536 => "npc_settimer";
    NPC_STAT = 2537 => "npc_stat";
    NPC_STATADD = 2538 => "npc_statadd";
    NPC_STATHEAL = 2539 => "npc_statheal";
    NPC_STATSUB = 2540 => "npc_statsub";
    NPC_TELE = 2541 => "npc_tele";
    NPC_TYPE = 2542 => "npc_type";
    NPC_UID = 2543 => "npc_uid";
    NPC_WALK = 2544 => "npc_walk";
    NPC_WALKTRIGGER = 2545 => "npc_walktrigger";
    PROJANIM_NPC = 2546 => "projanim_npc";
    SPOTANIM_NPC = 2547 => "spotanim_npc";
    NPC_DESTINATION = 2548 => "npc_destination";
    NPC_HUNTNEXT = 2549 => "npc_huntnext";
    NPC_PROJANIM_HALFSQ = 2550 => "npc_projanim_halfsq";
    NPC_QUEUEVARARG = 2551 => "npc_queuevararg";
    NPC_RUN = 2552 => "npc_run";

    // Loc
    LOC_ADD = 3000 => "loc_add";
    LOC_ANGLE = 3001 => "loc_angle";
    LOC_ANIM = 3002 => "loc_anim";
    LOC_CATEGORY = 3003 => "loc_category";
    LOC_CHANGE = 3004 => "loc_change";
    LOC_COORD = 3005 => "loc_coord";
    LOC_DEL = 3006 => "loc_del";
    LOC_FIND = 3007 => "loc_find";
    LOC_FINDALLZONE = 3008 => "loc_findallzone";
    LOC_FINDNEXT = 3009 => "loc_findnext";
    LOC_NAME = 3010 => "loc_name";
    LOC_PARAM = 3011 => "loc_param";
    LOC_SHAPE = 3012 => "loc_shape";
    LOC_TYPE = 3013 => "loc_type";
    LOC_PREFETCH = 3014 => "loc_prefetch";
    LOC_PREFETCHALL = 3015 => "loc_prefetchall";
    LOC_MERGE = 3016 => "loc_merge";

    // Obj
    OBJ_ADD = 3500 => "obj_add";
    OBJ_ADDALL = 3501 => "obj_addall";
    OBJ_COORD = 3502 => "obj_coord";
    OBJ_COUNT = 3503 => "obj_count";
    OBJ_DEL = 3504 => "obj_del";
    OBJ_FIND = 3505 => "obj_find";
    OBJ_FINDALLZONE = 3506 => "obj_findallzone";
    OBJ_FINDNEXT = 3507 => "obj_findnext";
    OBJ_NAME = 3508 => "obj_name";
    OBJ_PARAM = 3509 => "obj_param";
    OBJ_TAKEITEM = 3510 => "obj_takeitem";
    OBJ_TYPE = 3511 => "obj_type";
    OBJ_GETVAR = 3512 => "obj_getvar";
    OBJ_SETVAR = 3513 => "obj_setvar";

    // Npc config
    NC_CATEGORY = 4000 => "nc_category";
    NC_DEBUGNAME = 4001 => "nc_debugname";
    NC_DESC = 4002 => "nc_desc";
    NC_NAME = 4003 => "nc_name";
    NC_OP = 4004 => "nc_op";
    NC_PARAM = 4005 => "nc_param";
    NC_SIZE = 4006 => "nc_size";
    NC_VISLEVEL = 4007 => "nc_vislevel";

    // Loc config
    LC_CATEGORY = 4100 => "lc_category";
    LC_DEBUGNAME = 4101 => "lc_debugname";
    LC_DESC = 4102 => "lc_desc";
    LC_LENGTH = 4103 => "lc_length";
    LC_NAME = 4104 => "lc_name";
    LC_OP = 4105 => "lc_op";
    LC_PARAM = 4106 => "lc_param";
    LC_WIDTH = 4107 => "lc_width";

    // Obj config
    OC_CATEGORY = 4200 => "oc_category";
    OC_CERT = 4201 => "oc_cert";
    OC_COST = 4202 => "oc_cost";
    OC_DEBUGNAME = 4203 => "oc_debugname";
    OC_DESC = 4204 => "oc_desc";
    OC_IOP = 4205 => "oc_iop";
    OC_MEMBERS = 4206 => "oc_members";
    OC_NAME = 4207 => "oc_name";
    OC_OP = 4208 => "oc_op";
    OC_PARAM = 4209 => "oc_param";
    OC_STACKABLE = 4210 => "oc_stackable";
    OC_TRADEABLE = 4211 => "oc_tradeable";
    OC_UNCERT = 4212 => "oc_uncert";
    OC_WEARPOS = 4213 => "oc_wearpos";
    OC_WEARPOS2 = 4214 => "oc_wearpos2";
    OC_WEARPOS3 = 4215 => "oc_wearpos3";
    OC_WEIGHT = 4216 => "oc_weight";

    // Inventory
    BOTH_DROPSLOT = 4300 => "both_dropslot";
    BOTH_MOVEINV = 4301 => "both_moveinv";
    INV_ADD = 4302 => "inv_add";
    INV_ALLSTOCK = 4303 => "inv_allstock";
    INV_CHANGESLOT = 4304 => "inv_changeslot";
    INV_CLEAR = 4305 => "inv_clear";
    INV_DEBUGNAME = 4306 => "inv_debugname";
    INV_DEL = 4307 => "inv_del";
    INV_DELSLOT = 4308 => "inv_delslot";
    INV_DROPALL = 4309 => "inv_dropall";
    INV_DROPITEM_DELAYED = 4310 => "inv_dropitem_delayed";
    INV_DROPITEM = 4311 => "inv_dropitem";
    INV_DROPSLOT = 4312 => "inv_dropslot";
    INV_FREESPACE = 4313 => "inv_freespace";
    INV_GETNUM = 4314 => "inv_getnum";
    INV_GETOBJ = 4315 => "inv_getobj";
    INV_ITEMSPACE = 4316 => "inv_itemspace";
    INV_ITEMSPACE2 = 4317 => "inv_itemspace2";
    INV_MOVEFROMSLOT = 4318 => "inv_movefromslot";
    INV_MOVEITEM_CERT = 4319 => "inv_moveitem_cert";
    INV_MOVEITEM_UNCERT = 4320 => "inv_moveitem_uncert";
    INV_MOVEITEM = 4321 => "inv_moveitem";
    INV_MOVETOSLOT = 4322 => "inv_movetoslot";
    INV_SETSLOT = 4323 => "inv_setslot";
    INV_SIZE = 4324 => "inv_size";
    INV_STOCKBASE = 4325 => "inv_stockbase";
    INV_STOPTRANSMIT = 4326 => "inv_stoptransmit";
    INV_TOTAL = 4327 => "inv_total";
    INV_TOTALCAT = 4328 => "inv_totalcat";
    INV_TOTALPARAM_STACK = 4329 => "inv_totalparam_stack";
    INV_TOTALPARAM = 4330 => "inv_totalparam";
    INV_TRANSMIT = 4331 => "inv_transmit";
    INVOTHER_TRANSMIT = 4332 => "invother_transmit";
    INV_GETVAR = 4333 => "inv_getvar";
    INV_SETVAR = 4334 => "inv_setvar";
    INV_REORGANIZE = 4335 => "inv_reorganize";

    // Enum
    ENUM = 4400 => "enum";
    ENUM_GETOUTPUTCOUNT = 4401 => "enum_getoutputcount";

    // String
    APPEND_NUM = 4500 => "append_num";
    APPEND = 4501 => "append";
    APPEND_SIGNNUM = 4502 => "append_signnum";
    LOWERCASE = 4503 => "lowercase";
    TEXT_GENDER = 4504 => "text_gender";
    TOSTRING = 4505 => "tostring";
    COMPARE = 4506 => "compare";
    TEXT_SWITCH = 4507 => "text_switch";
    APPEND_CHAR = 4508 => "append_char";
    STRING_LENGTH = 4509 => "string_length";
    SUBSTRING = 4510 => "substring";
    STRING_INDEXOF_CHAR = 4511 => "string_indexof_char";
    STRING_INDEXOF_STRING = 4512 => "string_indexof_string";
    SPLIT_GET = 4513 => "split_get";
    SPLIT_GETANIM = 4514 => "split_getanim";
    SPLIT_INIT = 4515 => "split_init";
    SPLIT_LINECOUNT = 4516 => "split_linecount";
    SPLIT_PAGECOUNT = 4517 => "split_pagecount";

    // Number
    ADD = 4600 => "add";
    SUB = 4601 => "sub";
    MULTIPLY = 4602 => "multiply";
    DIVIDE = 4603 => "divide";
    RANDOM = 4604 => "random";
    RANDOMINC = 4605 => "randominc";
    INTERPOLATE = 4606 => "interpolate";
    ADDPERCENT = 4607 => "addpercent";
    SETBIT = 4608 => "setbit";
    CLEARBIT = 4609 => "clearbit";
    TESTBIT = 4610 => "testbit";
    MODULO = 4611 => "modulo";
    POW = 4612 => "pow";
    INVPOW = 4613 => "invpow";
    AND = 4614 => "and";
    OR = 4615 => "or";
    MIN = 4616 => "min";
    MAX = 4617 => "max";
    SCALE = 4618 => "scale";
    BITCOUNT = 4619 => "bitcount";
    TOGGLEBIT = 4620 => "togglebit";
    SETBIT_RANGE = 4621 => "setbit_range";
    CLEARBIT_RANGE = 4622 => "clearbit_range";
    GETBIT_RANGE = 4623 => "getbit_range";
    SETBIT_RANGE_TOINT = 4624 => "setbit_range_toint";
    SIN_DEG = 4625 => "sin_deg";
    COS_DEG = 4626 => "cos_deg";
    ATAN2_DEG = 4627 => "atan2_deg";
    ABS = 4628 => "abs";
    DATE_MINUTES = 4629 => "date_minutes";
    DATE_RUNEDAY = 4630 => "date_runeday";

    // Struct
    STRUCT_PARAM = 4700 => "struct_param";

    // Region
    REGION_FINDBYCOORD = 5200 => "region_findbycoord";
    REGION_GETCOORD = 5201 => "region_getcoord";
    REGION_CREATE = 5202 => "region_create";
    REGION_SET = 5203 => "region_set";
    REGION_UID = 5204 => "region_uid";
    REGION_FINDBYUID = 5205 => "region_findbyuid";
    REGION_SETEXITCOORD = 5206 => "region_setexitcoord";

    // Controller
    CONTROLLER_COORD = 5300 => "controller_coord";
    CONTROLLER_DEL = 5301 => "controller_del";
    CONTROLLER_QUEUE = 5302 => "controller_queue";
    CONTROLLER_FINDEXACT = 5303 => "controller_findexact";
    CONTROLLER_ADD = 5304 => "controller_add";
    CONTROLLER_SETTIMER = 5305 => "controller_settimer";
    CONTROLLER_QUEUEVARARG = 5306 => "controller_queuevararg";

    // Db
    DB_FIND_WITH_COUNT = 7500 => "db_find_with_count";
    DB_FINDNEXT = 7501 => "db_findnext";
    DB_GETFIELD = 7502 => "db_getfield";
    DB_GETFIELDCOUNT = 7503 => "db_getfieldcount";
    DB_LISTALL_WITH_COUNT = 7504 => "db_listall_with_count";
    DB_GETROWTABLE = 7505 => "db_getrowtable";
    DB_FINDBYINDEX = 7506 => "db_findbyindex";
    DB_FIND_REFINE_WITH_COUNT = 7507 => "db_find_refine_with_count";
    DB_FIND = 7508 => "db_find";
    DB_FIND_REFINE = 7509 => "db_find_refine";
    DB_LISTALL = 7510 => "db_listall";

    // Debug
    CONSOLE = 10000 => "console";
    ERROR = 10001 => "error";
    GETTIMESPENT = 10002 => "gettimespent";
    TIMESPENT = 10003 => "timespent";
    NPCCOUNT = 10004 => "npccount";
    ZONECOUNT = 10005 => "zonecount";
    LOCCOUNT = 10006 => "loccount";
    MAP_LASTCLOCK = 10007 => "map_lastclock";
    MAP_LASTWORLD = 10008 => "map_lastworld";
    MAP_LASTCLIENTIN = 10009 => "map_lastclientin";
    MAP_LASTNPC = 10010 => "map_lastnpc";
    MAP_LASTPLAYER = 10011 => "map_lastplayer";
    MAP_LASTLOGIN = 10012 => "map_lastlogin";
    MAP_LASTLOGOUT = 10013 => "map_lastlogout";
    MAP_LASTZONE = 10014 => "map_lastzone";
    MAP_LASTCLIENTOUT = 10015 => "map_lastclientout";
    MAP_LASTCLEANUP = 10016 => "map_lastcleanup";
    MAP_LASTBANDWIDTHIN = 10017 => "map_lastbandwidthin";
    MAP_LASTBANDWIDTHOUT = 10018 => "map_lastbandwidthout";
    MAP_PRODUCTION = 10019 => "map_production";
    OBJCOUNT = 10020 => "objcount";

    // Test framework
    ASSERT_EQ = 50000 => "assert_eq";
    ASSERT_NOT_EQ = 50001 => "assert_not_eq";
    ASSERT_STR_EQ = 50002 => "assert_str_eq";
    ASSERT_TRUE = 50003 => "assert_true";
    ASSERT_FALSE = 50004 => "assert_false";
    ASSERT_GT = 50005 => "assert_gt";
    ASSERT_LT = 50006 => "assert_lt";
    ASSERT_GTE = 50007 => "assert_gte";
    ASSERT_LTE = 50008 => "assert_lte";
    ASSERT_NULL = 50009 => "assert_null";
    ASSERT_NOT_NULL = 50010 => "assert_not_null";
    TEST_FAIL = 50011 => "test_fail";
    ASSERT_CALLED = 50012 => "assert_called";
    ASSERT_CALLED_WITH = 50013 => "assert_called_with";
    EXPECT_FAIL = 50014 => "expect_fail";
    TEST_TICK = 50050 => "test_tick";
    TEST_TRIGGER = 50051 => "test_trigger";
    TEST_FIRE_INPUT = 50052 => "test_fire_input";
    VERIFY_CALLS = 50053 => "verify_calls";
    WAIT_FOR = 50054 => "wait_for";
    TEST_OP = 50055 => "test_op";
    SETUP_PLAYER = 50100 => "setup_player";
}

/// The opcode a `command.pack` must give `command`.
pub fn opcode(command: &str) -> Option<u16> {
    COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|&(_, opcode)| opcode)
}

/// Whether `opcode`'s operand takes 4 bytes rather than 1.
pub fn is_large_operand(opcode: u16) -> bool {
    opcode <= 100
        && !matches!(
            opcode,
            RETURN | GOSUB | JUMP | POP_INT_DISCARD | POP_STRING_DISCARD
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn each_constant_is_its_command_in_upper_case() {
        for (constant, command) in CONSTANTS {
            assert_eq!(*constant, command.to_uppercase());
        }
    }

    #[test]
    fn each_name_and_each_constant_has_one_opcode() {
        let mut names = HashSet::new();
        for (name, _) in COMMANDS {
            assert!(names.insert(name), "{name}");
        }
        let mut opcodes = HashSet::new();
        for (_, command) in CONSTANTS {
            assert!(opcodes.insert(opcode(command)), "{command}");
        }
    }

    #[test]
    fn a_starred_name_shares_its_vararg_opcode() {
        assert_eq!(opcode("queue*"), Some(QUEUEVARARG));
        assert_eq!(opcode("queuevararg"), Some(QUEUEVARARG));
        assert_eq!(opcode("queue"), Some(QUEUE));
        assert_eq!(opcode("no_such_command"), None);
    }

    #[test]
    fn only_short_instructions_take_a_one_byte_operand() {
        assert!(is_large_operand(PUSH_CONSTANT_INT));
        assert!(is_large_operand(BRANCH));
        assert!(!is_large_operand(RETURN));
        assert!(!is_large_operand(POP_STRING_DISCARD));
        assert!(!is_large_operand(PUSH_CLANSETTING_INT));
        assert!(!is_large_operand(MES));
    }
}
