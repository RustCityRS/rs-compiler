//! Checked decoding using the host's wire-format description.
use super::{
    format::{Format, Width},
    invalid,
};
use crate::bytecode::Opcode;
use crate::types::BaseVarType;
use std::io::Result;

#[derive(Clone, Debug)]
pub struct Script {
    pub name: String,
    pub local_int_count: u16,
    pub local_string_count: u16,
    pub local_long_count: u16,
    pub int_param_count: u16,
    pub string_param_count: u16,
    pub long_param_count: u16,
    /// Physical opcode IDs are retained for disassembly and lossless output.
    pub opcodes: Vec<u16>,
    pub operations: Vec<Opcode>,
    pub int_operands: Vec<i32>,
    pub string_operands: Vec<Option<String>>,
    pub long_operands: Vec<Option<i64>>,
    pub switches: Vec<Vec<(i32, i32)>>,
}

pub fn is_branch(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Branch
            | Opcode::BranchNot
            | Opcode::BranchEquals
            | Opcode::BranchLessThan
            | Opcode::BranchGreaterThan
            | Opcode::BranchLessThanOrEquals
            | Opcode::BranchGreaterThanOrEquals
            | Opcode::LongBranchNot
            | Opcode::LongBranchEquals
            | Opcode::LongBranchLessThan
            | Opcode::LongBranchGreaterThan
            | Opcode::LongBranchLessThanOrEquals
            | Opcode::LongBranchGreaterThanOrEquals
            | Opcode::BranchIfTrue
            | Opcode::BranchIfFalse
            | Opcode::ObjBranchEquals
            | Opcode::ObjBranchNot
    )
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| invalid("Byte offset overflow"))?;
        let bytes = self.bytes.get(self.offset..end).ok_or_else(|| {
            invalid(format!(
                "Truncated bytecode at byte {}: expected {count} bytes",
                self.offset
            ))
        })?;
        self.offset = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let len = self.bytes[self.offset..]
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| invalid("Unterminated bytecode string"))?;
        let bytes = self.take(len + 1)?;
        const EXTENDED: [char; 32] = [
            '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
            '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
            '\u{9d}', 'ž', 'Ÿ',
        ];
        Ok(bytes[..len]
            .iter()
            .map(|&byte| {
                if (128..160).contains(&byte) {
                    EXTENDED[(byte - 128) as usize]
                } else {
                    byte as char
                }
            })
            .collect())
    }
    fn finished(&self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid(format!(
                "{} trailing bytecode bytes",
                self.bytes.len() - self.offset
            )))
        }
    }
}

impl Script {
    pub fn decode(data: &[u8], format: &Format) -> Result<Self> {
        let fixed = 4 + 4 * format.local_types.len();
        let (metadata_end, switch_size) = if format.switches {
            let tail = data
                .get(data.len().saturating_sub(2)..)
                .filter(|tail| tail.len() == 2)
                .ok_or_else(|| invalid("Missing switch footer length"))?;
            let size = u16::from_be_bytes(tail.try_into().unwrap()) as usize;
            if size == 0 {
                return Err(invalid("Missing switch count"));
            }
            (data.len() - 2, size)
        } else {
            (data.len(), 0)
        };
        let start = metadata_end
            .checked_sub(fixed + switch_size)
            .ok_or_else(|| invalid("Truncated bytecode footer"))?;
        let mut metadata = Reader::new(&data[start..metadata_end]);
        let count = metadata.i32()?;
        if count < 0 || count as usize > start / 3 {
            return Err(invalid(format!("Invalid instruction count {count}")));
        }
        let mut counts = [[0; 3]; 2];
        for counts in &mut counts {
            let mut seen = [false; 3];
            for kind in &format.local_types {
                let index = match kind {
                    BaseVarType::Integer => 0,
                    BaseVarType::String => 1,
                    BaseVarType::Long => 2,
                };
                if seen[index] {
                    return Err(invalid("Repeated local stack in format"));
                }
                seen[index] = true;
                counts[index] = metadata.u16()?;
            }
        }
        if (0..3).any(|i| counts[1][i] > counts[0][i]) {
            return Err(invalid("Parameters exceed local counts"));
        }
        let mut switches = Vec::new();
        if format.switches {
            let tables = metadata.u8()?;
            for _ in 0..tables {
                let entries = metadata.u16()? as usize;
                let mut table = Reader::new(metadata.take(entries * 8)?);
                let mut cases = Vec::with_capacity(entries);
                for _ in 0..entries {
                    cases.push((table.i32()?, table.i32()?));
                }
                switches.push(cases);
            }
        }
        metadata.finished()?;
        let mut code = Reader::new(&data[..start]);
        let mut script = Self {
            name: code.string()?,
            local_int_count: counts[0][0],
            local_string_count: counts[0][1],
            local_long_count: counts[0][2],
            int_param_count: counts[1][0],
            string_param_count: counts[1][1],
            long_param_count: counts[1][2],
            opcodes: Vec::with_capacity(count as usize),
            operations: Vec::with_capacity(count as usize),
            int_operands: Vec::with_capacity(count as usize),
            string_operands: Vec::with_capacity(count as usize),
            long_operands: Vec::with_capacity(count as usize),
            switches,
        };
        for pc in 0..count as usize {
            let opcode = code.u16()?;
            let (operation, width) = format
                .decode(opcode)
                .map_err(|error| invalid(format!("Instruction {pc}: {error}")))?;
            let mut integer = 0;
            let mut string = None;
            let mut long = None;
            match width {
                Width::Byte => integer = code.u8()? as i32,
                Width::Int => integer = code.i32()?,
                Width::Long => long = Some(code.i64()?),
                Width::String => string = Some(code.string()?),
            }
            script.opcodes.push(opcode);
            script.operations.push(operation);
            script.int_operands.push(integer);
            script.string_operands.push(string);
            script.long_operands.push(long);
        }
        code.finished()?;
        for (pc, &operation) in script.operations.iter().enumerate() {
            let operand = script.int_operands[pc];
            let check_target = |offset: i32| -> Result<()> {
                let target = pc as i64 + 1 + offset as i64;
                if !(0..=count as i64).contains(&target) {
                    return Err(invalid(format!(
                        "Instruction {pc}: branch target {target} outside script"
                    )));
                }
                Ok(())
            };
            if is_branch(operation) {
                check_target(operand)?;
            }
            if operation == Opcode::Switch {
                let table = script.switches.get(operand as usize).ok_or_else(|| {
                    invalid(format!("Instruction {pc}: invalid switch table {operand}"))
                })?;
                for &(_, offset) in table {
                    check_target(offset)?;
                }
            }
            let local = match operation {
                Opcode::PushIntLocal | Opcode::PopIntLocal => Some(script.local_int_count),
                Opcode::PushStringLocal | Opcode::PopStringLocal => Some(script.local_string_count),
                Opcode::PushLongLocal | Opcode::PopLongLocal => Some(script.local_long_count),
                _ => None,
            };
            if local.is_some_and(|count| operand < 0 || operand >= count as i32) {
                return Err(invalid(format!(
                    "Instruction {pc}: local slot {operand} outside declared locals"
                )));
            }
        }
        Ok(script)
    }
}
