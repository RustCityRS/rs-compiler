use super::invalid;
use crate::bytecode::Opcode;
use crate::types::BaseVarType;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Result;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Int,
    Long,
    String,
}

impl Width {
    fn instruction(opcode: Opcode) -> Self {
        match opcode {
            Opcode::PushConstantString => Self::String,
            Opcode::PushConstantLong => Self::Long,
            Opcode::Return | Opcode::PopIntDiscard | Opcode::PopStringDiscard => Self::Byte,
            _ => Self::Int,
        }
    }

    pub(super) fn integer(self, out: &mut Vec<u8>, value: i64) -> Result<()> {
        match self {
            Self::Byte => out.push(u8::try_from(value).map_err(invalid)?),
            Self::Int => {
                out.extend_from_slice(&i32::try_from(value).map_err(invalid)?.to_be_bytes())
            }
            Self::Long => out.extend_from_slice(&value.to_be_bytes()),
            Self::String => return Err(invalid("Expected string operand")),
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Format {
    instructions: HashMap<Opcode, (u16, Width)>,
    decoding: HashMap<u16, (Opcode, Width)>,
    pub commands: BTreeMap<u16, String>,
    pub local_types: Vec<BaseVarType>,
    pub switches: bool,
}

impl Format {
    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&fs::read_to_string(path)?)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut instructions = HashMap::new();
        let mut commands = BTreeMap::new();
        let mut codes = HashSet::new();
        let mut names = HashSet::new();
        for (index, line) in text.lines().map(str::trim).enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (code, name) = line
                .split_once('=')
                .ok_or_else(|| invalid(format!("Line {}: expected opcode=name", index + 1)))?;
            let (code, name) = (code.trim().parse::<u16>().map_err(invalid)?, name.trim());
            if name.is_empty() {
                return Err(invalid(format!("Missing operation name for opcode {code}")));
            }
            if !codes.insert(code) || !names.insert(name) {
                return Err(invalid(format!(
                    "Duplicate operation {name} or opcode {code}"
                )));
            }
            if let Some(opcode) = Opcode::core(name) {
                instructions.insert(opcode, (code, Width::instruction(opcode)));
            } else {
                commands.insert(code, name.to_owned());
            }
        }
        let mut local_types = vec![BaseVarType::Integer, BaseVarType::String];
        if instructions.contains_key(&Opcode::PushConstantLong) {
            local_types.push(BaseVarType::Long);
        }
        let switches = instructions.contains_key(&Opcode::Switch);
        let mut decoding = instructions
            .iter()
            .map(|(&opcode, &(code, width))| (code, (opcode, width)))
            .collect::<HashMap<_, _>>();
        decoding.extend(
            commands
                .keys()
                .map(|&code| (code, (Opcode::Command, Width::Byte))),
        );
        Ok(Self {
            instructions,
            decoding,
            commands,
            local_types,
            switches,
        })
    }

    pub fn instruction(&self, opcode: Opcode) -> Result<(u16, Width)> {
        self.instructions
            .get(&opcode)
            .copied()
            .ok_or_else(|| invalid(format!("Missing operation {opcode:?} in opcode pack")))
    }

    pub fn width(&self, code: u16) -> Width {
        self.decoding
            .get(&code)
            .map_or(Width::Byte, |&(_, width)| width)
    }

    /// Define a wire operation explicitly, including nonstandard operand widths.
    pub fn define(&mut self, operation: Opcode, code: u16, width: Width) -> Result<()> {
        let compatible = match operation {
            Opcode::PushConstantString => width == Width::String,
            Opcode::PushConstantLong => width == Width::Long,
            _ => matches!(width, Width::Byte | Width::Int),
        };
        if !compatible {
            return Err(invalid(format!(
                "Operand width {width:?} is incompatible with {operation:?}"
            )));
        }
        if self.decoding.contains_key(&code) || self.instructions.contains_key(&operation) {
            return Err(invalid(format!(
                "Duplicate operation {operation:?} or opcode {code}"
            )));
        }
        self.instructions.insert(operation, (code, width));
        self.decoding.insert(code, (operation, width));
        Ok(())
    }

    pub fn define_command(&mut self, name: String, code: u16, width: Width) -> Result<()> {
        if name.is_empty() || !matches!(width, Width::Byte | Width::Int) {
            return Err(invalid(
                "Commands require a name and a byte or integer operand",
            ));
        }
        if self.decoding.contains_key(&code)
            || self.commands.values().any(|existing| *existing == name)
        {
            return Err(invalid(format!(
                "Duplicate command {name} or opcode {code}"
            )));
        }
        self.commands.insert(code, name);
        self.decoding.insert(code, (Opcode::Command, width));
        Ok(())
    }

    pub fn decode(&self, code: u16) -> Result<(Opcode, Width)> {
        self.decoding
            .get(&code)
            .copied()
            .ok_or_else(|| invalid(format!("Undefined opcode {code}")))
    }
}
