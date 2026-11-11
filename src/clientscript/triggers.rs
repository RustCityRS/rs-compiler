//! Client trigger signatures shared by compilation and editor analysis.
use crate::types::Type;

pub(crate) struct Trigger {
    pub name: &'static str,
    pub subject_type: Option<Type>,
    pub params: Option<&'static [Type]>,
}

impl Trigger {
    pub fn subject<'a>(&self, name: &'a str) -> Option<(&'a str, Type)> {
        let ty = self.subject_type?;
        if name == "_" {
            None
        } else if let Some(name) = name.strip_prefix('_') {
            Some((name, Type::Category))
        } else {
            Some((name, ty))
        }
    }
}

const fn event(name: &'static str, subject_type: Type, params: &'static [Type]) -> Trigger {
    Trigger {
        name,
        subject_type: Some(subject_type),
        params: Some(params),
    }
}

pub(crate) const TRIGGERS: &[Trigger] = &[
    Trigger {
        name: "proc",
        subject_type: None,
        params: None,
    },
    Trigger {
        name: "clientscript",
        subject_type: None,
        params: None,
    },
    event("opworldmapelement1", Type::MapElement, &[]),
    event("opworldmapelement2", Type::MapElement, &[]),
    event("opworldmapelement3", Type::MapElement, &[]),
    event("opworldmapelement4", Type::MapElement, &[]),
    event("opworldmapelement5", Type::MapElement, &[]),
    event(
        "worldmapelementmouseover",
        Type::MapElement,
        &[Type::Int, Type::Int],
    ),
    event(
        "worldmapelementmouseleave",
        Type::MapElement,
        &[Type::Int, Type::Int],
    ),
    event(
        "worldmapelementmouserepeat",
        Type::MapElement,
        &[Type::Int, Type::Int],
    ),
    event("loadnpc", Type::Npc, &[]),
    event("loadloc", Type::Loc, &[]),
];

pub(crate) fn lookup(name: &str) -> Option<&'static Trigger> {
    TRIGGERS.iter().find(|trigger| trigger.name == name)
}
