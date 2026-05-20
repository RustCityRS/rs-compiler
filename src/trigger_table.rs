//! Single source of truth for RuneScript trigger metadata.
//!
//! Every trigger keyword (`[opnpc1,…]`, `[if_button,…]`, `[login,_]`, …)
//! has the same shape of metadata: a byte discriminant for lookup-key
//! dispatch, an optional preferred subject type for entity resolution,
//! whether its subject should be validated to resolve, and whether it's a
//! button trigger that participates in pointer-grant logic.
//!
//! Before this module existed, that metadata was scattered across three
//! separate matches in `compiler.rs` (the byte table, the
//! `is_byte_keyed_trigger` whitelist, and the `preferred_type` selector)
//! plus a 13-arm match in `pointer_checker.rs`. Adding a trigger meant
//! editing four places, and one of them was the bug shape that silently
//! produced unreachable scripts.

use crate::types::Type;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub struct TriggerInfo {
    pub name: &'static str,
    pub byte: u8,
    /// Preferred entity type for `lookup_entity_id_typed`. `None` falls
    /// through to the flat entity-id lookup.
    pub subject_type: Option<Type>,
    /// True if a `[trigger,name]` script with a non-`_`, non-`_category`,
    /// non-coord subject should resolve to a real entity. Drives the
    /// "subject did not resolve" warning. False for triggers that:
    ///   - have no entity subject (login, logout)
    ///   - take coord-shaped subjects (zone, zoneexit, mapzone, mapzoneexit)
    ///   - intentionally don't validate (ai_queue, ai_timer — preserved
    ///     from the original `is_byte_keyed_trigger` whitelist; widening
    ///     this would change emitted warnings)
    pub validates_subject: bool,
    /// True for if_button/if_button1-5/if_buttond and inv_button1-5/inv_buttond.
    /// Used by pointer_checker to grant p_active_player on non-overlay
    /// interfaces, mirroring ServerPointerChecker.setsPointerTrigger().
    pub is_button: bool,
}

const TRIGGERS: &[TriggerInfo] = &[
    // ── NPC triggers ─────────────────────────────────────────────────
    npc("apnpc1", 3),
    npc("apnpc2", 4),
    npc("apnpc3", 5),
    npc("apnpc4", 6),
    npc("apnpc5", 7),
    npc("apnpcu", 8),
    npc("apnpct", 9),
    npc("opnpc1", 10),
    npc("opnpc2", 11),
    npc("opnpc3", 12),
    npc("opnpc4", 13),
    npc("opnpc5", 14),
    npc("opnpcu", 15),
    npc("opnpct", 16),
    npc("ai_apnpc1", 17),
    npc("ai_apnpc2", 18),
    npc("ai_apnpc3", 19),
    npc("ai_apnpc4", 20),
    npc("ai_apnpc5", 21),
    npc("ai_opnpc1", 24),
    npc("ai_opnpc2", 25),
    npc("ai_opnpc3", 26),
    npc("ai_opnpc4", 27),
    npc("ai_opnpc5", 28),
    // ── Ground obj triggers ──────────────────────────────────────────
    obj("apobj1", 31),
    obj("apobj2", 32),
    obj("apobj3", 33),
    obj("apobj4", 34),
    obj("apobj5", 35),
    obj("apobju", 36),
    obj("apobjt", 37),
    obj("opobj1", 38),
    obj("opobj2", 39),
    obj("opobj3", 40),
    obj("opobj4", 41),
    obj("opobj5", 42),
    obj("opobju", 43),
    obj("opobjt", 44),
    obj("ai_apobj1", 45),
    obj("ai_apobj2", 46),
    obj("ai_apobj3", 47),
    obj("ai_apobj4", 48),
    obj("ai_apobj5", 49),
    obj("ai_opobj1", 52),
    obj("ai_opobj2", 53),
    obj("ai_opobj3", 54),
    obj("ai_opobj4", 55),
    obj("ai_opobj5", 56),
    // ── Loc triggers ─────────────────────────────────────────────────
    loc("aploc1", 59),
    loc("aploc2", 60),
    loc("aploc3", 61),
    loc("aploc4", 62),
    loc("aploc5", 63),
    loc("aplocu", 64),
    loc("aploct", 65),
    loc("oploc1", 66),
    loc("oploc2", 67),
    loc("oploc3", 68),
    loc("oploc4", 69),
    loc("oploc5", 70),
    loc("oplocu", 71),
    loc("oploct", 72),
    loc("ai_aploc1", 73),
    loc("ai_aploc2", 74),
    loc("ai_aploc3", 75),
    loc("ai_aploc4", 76),
    loc("ai_aploc5", 77),
    loc("ai_oploc1", 80),
    loc("ai_oploc2", 81),
    loc("ai_oploc3", 82),
    loc("ai_oploc4", 83),
    loc("ai_oploc5", 84),
    // ── Player triggers (no preferred type — fall through to flat lookup) ──
    flat("applayer1", 87),
    flat("applayer2", 88),
    flat("applayer3", 89),
    flat("applayer4", 90),
    flat("applayer5", 91),
    flat("applayeru", 92),
    flat("applayert", 93),
    flat("opplayer1", 94),
    flat("opplayer2", 95),
    flat("opplayer3", 96),
    flat("opplayer4", 97),
    flat("opplayer5", 98),
    flat("opplayeru", 99),
    flat("opplayert", 100),
    flat("ai_applayer1", 101),
    flat("ai_applayer2", 102),
    flat("ai_applayer3", 103),
    flat("ai_applayer4", 104),
    flat("ai_applayer5", 105),
    flat("ai_opplayer1", 108),
    flat("ai_opplayer2", 109),
    flat("ai_opplayer3", 110),
    flat("ai_opplayer4", 111),
    flat("ai_opplayer5", 112),
    // ── AI queue / timer (NPC-keyed, but historically not validated) ──
    npc_no_validate("ai_queue1", 117),
    npc_no_validate("ai_queue2", 118),
    npc_no_validate("ai_queue3", 119),
    npc_no_validate("ai_queue4", 120),
    npc_no_validate("ai_queue5", 121),
    npc_no_validate("ai_queue6", 122),
    npc_no_validate("ai_queue7", 123),
    npc_no_validate("ai_queue8", 124),
    npc_no_validate("ai_queue9", 125),
    npc_no_validate("ai_queue10", 126),
    npc_no_validate("ai_queue11", 127),
    npc_no_validate("ai_queue12", 128),
    npc_no_validate("ai_queue13", 129),
    npc_no_validate("ai_queue14", 130),
    npc_no_validate("ai_queue15", 131),
    npc_no_validate("ai_queue16", 132),
    npc_no_validate("ai_queue17", 133),
    npc_no_validate("ai_queue18", 134),
    npc_no_validate("ai_queue19", 135),
    npc_no_validate("ai_queue20", 136),
    flat_no_validate("ai_timer", 139),
    // ── Held / inventory item triggers ───────────────────────────────
    obj("opheld1", 140),
    obj("opheld2", 141),
    obj("opheld3", 142),
    obj("opheld4", 143),
    obj("opheld5", 144),
    obj("opheldu", 145),
    obj("opheldt", 146),
    // ── Interface / button triggers ──────────────────────────────────
    iface("if_button", 147, true),
    iface("if_close", 148, false),
    iface("if_button1", 149, true),
    iface("if_button2", 150, true),
    iface("if_button3", 151, true),
    iface("if_button4", 152, true),
    iface("if_button5", 153, true),
    iface("if_buttond", 154, true),
    component("inv_button1", 149),
    component("inv_button2", 150),
    component("inv_button3", 151),
    component("inv_button4", 152),
    component("inv_button5", 153),
    component("inv_buttond", 154),
    // ── Player lifecycle / coord / stat triggers ─────────────────────
    flat_no_validate("login", 157),
    flat_no_validate("logout", 158),
    flat("tutorial", 159),
    flat("advancestat", 160),
    flat_no_validate("mapzone", 161),
    flat_no_validate("mapzoneexit", 162),
    flat_no_validate("zone", 163),
    flat_no_validate("zoneexit", 164),
    flat("changestat", 165),
    flat("ai_spawn", 166),
    flat("ai_despawn", 167),
];

// ── Constructor helpers ──────────────────────────────────────────────
//
// `const fn` so the static slice above stays trivially constructible.

const fn npc(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: Some(Type::Npc),
        validates_subject: true,
        is_button: false,
    }
}

const fn npc_no_validate(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: Some(Type::Npc),
        validates_subject: false,
        is_button: false,
    }
}

const fn obj(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: Some(Type::Obj),
        validates_subject: true,
        is_button: false,
    }
}

const fn loc(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: Some(Type::Loc),
        validates_subject: true,
        is_button: false,
    }
}

const fn iface(name: &'static str, byte: u8, is_button: bool) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: Some(Type::Interface),
        validates_subject: true,
        is_button,
    }
}

const fn component(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: Some(Type::Component),
        validates_subject: true,
        is_button: true,
    }
}

const fn flat(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: None,
        validates_subject: true,
        is_button: false,
    }
}

const fn flat_no_validate(name: &'static str, byte: u8) -> TriggerInfo {
    TriggerInfo {
        name,
        byte,
        subject_type: None,
        validates_subject: false,
        is_button: false,
    }
}

// ── Name-keyed triggers ─────────────────────────────────────────────
//
// Triggers dispatched by script name (no byte discriminant). These are
// valid trigger keywords but don't participate in lookup-key hashing.

const NAME_KEYED_TRIGGERS: &[&str] = &[
    "proc",
    "label",
    "debugproc",
    "command",
    "clientscript",
    "walktrigger",
    "queue",
    "timer",
    "softtimer",
];

// Rev-647 extensions: valid triggers whose byte discriminants are not
// yet standardized in the reference implementation.
const EXTENDED_TRIGGERS: &[&str] = &["opheld6", "opheld7", "opheld8", "switch_window_mode"];

// ── Public lookup API ────────────────────────────────────────────────

fn table() -> &'static HashMap<&'static str, TriggerInfo> {
    static TABLE: OnceLock<HashMap<&'static str, TriggerInfo>> = OnceLock::new();
    TABLE.get_or_init(|| TRIGGERS.iter().map(|t| (t.name, *t)).collect())
}

pub fn lookup(name: &str) -> Option<TriggerInfo> {
    table().get(name).copied()
}

pub fn byte(name: &str) -> Option<u8> {
    lookup(name).map(|t| t.byte)
}

pub fn subject_type(name: &str) -> Option<Type> {
    lookup(name).and_then(|t| t.subject_type)
}

pub fn validates_subject(name: &str) -> bool {
    lookup(name).map(|t| t.validates_subject).unwrap_or(false)
}

pub fn is_button(name: &str) -> bool {
    lookup(name).map(|t| t.is_button).unwrap_or(false)
}

pub fn is_valid_trigger(name: &str) -> bool {
    lookup(name).is_some()
        || NAME_KEYED_TRIGGERS.contains(&name)
        || EXTENDED_TRIGGERS.contains(&name)
}

pub fn allows_returns(name: &str) -> bool {
    matches!(name, "proc" | "clientscript" | "command" | "logout")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_trigger_names() {
        let mut seen = std::collections::HashSet::new();
        for t in TRIGGERS {
            assert!(seen.insert(t.name), "duplicate trigger name: {}", t.name);
        }
    }

    #[test]
    fn button_triggers_match_legacy_set() {
        // Locks in pointer_checker's old hand-maintained list.
        let expected = [
            "if_button",
            "if_button1",
            "if_button2",
            "if_button3",
            "if_button4",
            "if_button5",
            "if_buttond",
            "inv_button1",
            "inv_button2",
            "inv_button3",
            "inv_button4",
            "inv_button5",
            "inv_buttond",
        ];
        for name in expected {
            assert!(is_button(name), "{} should be a button trigger", name);
        }
        // Spot-check that non-buttons aren't flagged.
        assert!(!is_button("if_close"));
        assert!(!is_button("opnpc1"));
        assert!(!is_button("login"));
    }

    #[test]
    fn known_byte_values() {
        // Anchors against the RuneScriptTS ServerTriggerType.ts byte
        // discriminants. If any of these change, the engine's dispatch
        // will silently desync.
        assert_eq!(byte("opnpc1"), Some(10));
        assert_eq!(byte("opheld1"), Some(140));
        assert_eq!(byte("if_button"), Some(147));
        assert_eq!(byte("if_button1"), Some(149));
        assert_eq!(byte("inv_button1"), Some(149));
        assert_eq!(byte("login"), Some(157));
        assert_eq!(byte("ai_despawn"), Some(167));
        assert_eq!(byte("totally_unknown_trigger"), None);
    }

    #[test]
    fn subject_type_routing() {
        assert_eq!(subject_type("opnpc1"), Some(Type::Npc));
        assert_eq!(subject_type("opobj1"), Some(Type::Obj));
        assert_eq!(subject_type("oploc1"), Some(Type::Loc));
        assert_eq!(subject_type("opheld1"), Some(Type::Obj));
        assert_eq!(subject_type("if_button"), Some(Type::Interface));
        assert_eq!(subject_type("inv_button1"), Some(Type::Component));
        assert_eq!(subject_type("login"), None);
        assert_eq!(subject_type("opplayer1"), None); // flat lookup
    }

    #[test]
    fn validation_skip_list() {
        // Triggers historically excluded from subject validation.
        assert!(!validates_subject("login"));
        assert!(!validates_subject("logout"));
        assert!(!validates_subject("zone"));
        assert!(!validates_subject("zoneexit"));
        assert!(!validates_subject("mapzone"));
        assert!(!validates_subject("mapzoneexit"));
        assert!(!validates_subject("ai_queue1"));
        assert!(!validates_subject("ai_timer"));
        // …but the rest of the byte-keyed triggers do validate.
        assert!(validates_subject("if_button"));
        assert!(validates_subject("opnpc1"));
        assert!(validates_subject("opheld1"));
        assert!(validates_subject("tutorial"));
    }

    #[test]
    fn is_valid_trigger_byte_keyed() {
        assert!(is_valid_trigger("opnpc1"));
        assert!(is_valid_trigger("opheld1"));
        assert!(is_valid_trigger("if_button"));
        assert!(is_valid_trigger("login"));
        assert!(is_valid_trigger("ai_queue1"));
        assert!(is_valid_trigger("ai_despawn"));
    }

    #[test]
    fn is_valid_trigger_name_keyed() {
        assert!(is_valid_trigger("proc"));
        assert!(is_valid_trigger("label"));
        assert!(is_valid_trigger("debugproc"));
        assert!(is_valid_trigger("command"));
        assert!(is_valid_trigger("clientscript"));
        assert!(is_valid_trigger("walktrigger"));
        assert!(is_valid_trigger("queue"));
        assert!(is_valid_trigger("timer"));
        assert!(is_valid_trigger("softtimer"));
    }

    #[test]
    fn is_valid_trigger_extended() {
        assert!(is_valid_trigger("opheld6"));
        assert!(is_valid_trigger("opheld7"));
        assert!(is_valid_trigger("opheld8"));
        assert!(is_valid_trigger("switch_window_mode"));
    }

    #[test]
    fn is_valid_trigger_unknown_rejected() {
        assert!(!is_valid_trigger("not_a_real_trigger"));
        assert!(!is_valid_trigger(""));
        assert!(!is_valid_trigger("procc"));
    }

    #[test]
    fn allows_returns_matches_lib_set() {
        assert!(allows_returns("proc"));
        assert!(allows_returns("clientscript"));
        assert!(allows_returns("command"));
        assert!(allows_returns("logout"));
        assert!(!allows_returns("label"));
        assert!(!allows_returns("debugproc"));
        assert!(!allows_returns("queue"));
        assert!(!allows_returns("timer"));
        assert!(!allows_returns("softtimer"));
        assert!(!allows_returns("walktrigger"));
        assert!(!allows_returns("login"));
        assert!(!allows_returns("opnpc1"));
    }

    #[test]
    fn name_keyed_triggers_have_no_byte() {
        assert_eq!(byte("proc"), None);
        assert_eq!(byte("label"), None);
        assert_eq!(byte("command"), None);
        assert_eq!(byte("walktrigger"), None);
    }
}
