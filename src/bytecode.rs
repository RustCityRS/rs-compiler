/// RS2 opcodes matching the actual wire format.
/// These values correspond to the opcodes used by the RuneScape 2 script engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    PushConstantInt = 0,
    PushVarp = 1,
    PopVarp = 2,
    PushConstantString = 3,
    PushVarn = 4,
    PopVarn = 5,
    Branch = 6,
    BranchNot = 7,
    BranchEquals = 8,
    BranchLessThan = 9,
    BranchGreaterThan = 10,
    PushVars = 11,
    PopVars = 12,
    Return = 21,
    Gosub = 22,
    Jump = 23,
    Switch = 24,
    PushVarbit = 25,
    PopVarbit = 27,
    BranchLessThanOrEquals = 31,
    BranchGreaterThanOrEquals = 32,
    PushIntLocal = 33,
    PopIntLocal = 34,
    PushStringLocal = 35,
    PopStringLocal = 36,
    JoinString = 37,
    PopIntDiscard = 38,
    PopStringDiscard = 39,
    GosubWithParams = 40,
    JumpWithParams = 41,
    DefineArray = 44,
    PushArrayInt = 45,
    PopArrayInt = 46,
    PushConstantLong = 54,
    PushLongLocal = 55,
    PopLongLocal = 56,
    PopLongDiscard = 57,
    // Long branch operations
    LongBranchNot = 68,
    LongBranchEquals = 69,
    LongBranchLessThan = 70,
    LongBranchGreaterThan = 71,
    LongBranchLessThanOrEquals = 72,
    LongBranchGreaterThanOrEquals = 73,
    // Object/string branch operations
    ObjBranchEquals = 86,
    ObjBranchNot = 87,
    // Arithmetic operations (4600+ range — engine commands)
    Add = 4600,
    Sub = 4601,
    Multiply = 4602,
    Divide = 4603,
    Random = 4604,
    RandomInc = 4605,
    Interpolate = 4606,
    AddPercent = 4607,
    SetBit = 4608,
    ClearBit = 4609,
    TestBit = 4610,
    Modulo = 4611,
    Pow = 4612,
    InvPow = 4613,
    And = 4614,
    Or = 4615,
    Min = 4616,
    Max = 4617,
    Scale = 4618,
    BitCount = 4619,
    ToggleBit = 4620,
    SetBitRange = 4621,
    ClearBitRange = 4622,
    GetBitRange = 4623,
    SetBitRangeToVal = 4624,
    SinDeg = 4625,
    CosDeg = 4626,
    Atan2Deg = 4627,
    Abs = 4628,
    // String operations (4200+ range)
    // Command is used for engine commands - opcode is looked up dynamically
    Command = 65535,
    // Line number metadata (not a real opcode, used for debugging)
    LineNumber = 65534,
}

/// A single instruction in the compiled bytecode.
#[derive(Debug, Clone)]
pub struct Instruction {
    pub opcode: Opcode,
    pub operand: Operand,
    pub line: Option<u32>,
}

/// The operand (data) associated with an instruction.
#[derive(Debug, Clone)]
pub enum Operand {
    None,
    Int(i32),
    Long(i64),
    Str(String),
    /// Jump target as instruction index within the script
    JumpTarget(usize),
    /// Switch table: list of (case_value, jump_target) pairs
    SwitchTable(Vec<(i32, usize)>),
    /// String join with N parts
    StringCount(u32),
    /// Array definition: (var_id, element_type_char)
    ArrayDef(i32, u8),
}

impl Instruction {
    pub fn new(opcode: Opcode, operand: Operand) -> Self {
        Instruction {
            opcode,
            operand,
            line: None,
        }
    }

    pub fn with_line(opcode: Opcode, operand: Operand, line: u32) -> Self {
        Instruction {
            opcode,
            operand,
            line: Some(line),
        }
    }

    pub fn simple(opcode: Opcode) -> Self {
        Instruction {
            opcode,
            operand: Operand::None,
            line: None,
        }
    }

    pub fn push_int(value: i32) -> Self {
        Instruction::new(Opcode::PushConstantInt, Operand::Int(value))
    }

    pub fn push_string(value: String) -> Self {
        Instruction::new(Opcode::PushConstantString, Operand::Str(value))
    }

    pub fn push_long(value: i64) -> Self {
        Instruction::new(Opcode::PushConstantLong, Operand::Long(value))
    }

    pub fn jump(target: usize) -> Self {
        Instruction::new(Opcode::Branch, Operand::JumpTarget(target))
    }

    pub fn branch_not(target: usize) -> Self {
        Instruction::new(Opcode::BranchNot, Operand::JumpTarget(target))
    }

    pub fn branch_equals(target: usize) -> Self {
        Instruction::new(Opcode::BranchEquals, Operand::JumpTarget(target))
    }

    pub fn branch_less_than(target: usize) -> Self {
        Instruction::new(Opcode::BranchLessThan, Operand::JumpTarget(target))
    }

    pub fn branch_greater_than(target: usize) -> Self {
        Instruction::new(Opcode::BranchGreaterThan, Operand::JumpTarget(target))
    }

    pub fn branch_less_than_or_equals(target: usize) -> Self {
        Instruction::new(Opcode::BranchLessThanOrEquals, Operand::JumpTarget(target))
    }

    pub fn branch_greater_than_or_equals(target: usize) -> Self {
        Instruction::new(
            Opcode::BranchGreaterThanOrEquals,
            Operand::JumpTarget(target),
        )
    }

    pub fn gosub(script_id: i32) -> Self {
        Instruction::new(Opcode::Gosub, Operand::Int(script_id))
    }

    pub fn gosub_with_params(script_id: i32) -> Self {
        Instruction::new(Opcode::GosubWithParams, Operand::Int(script_id))
    }

    pub fn push_int_local(var_id: i32) -> Self {
        Instruction::new(Opcode::PushIntLocal, Operand::Int(var_id))
    }

    pub fn pop_int_local(var_id: i32) -> Self {
        Instruction::new(Opcode::PopIntLocal, Operand::Int(var_id))
    }

    pub fn push_string_local(var_id: i32) -> Self {
        Instruction::new(Opcode::PushStringLocal, Operand::Int(var_id))
    }

    pub fn pop_string_local(var_id: i32) -> Self {
        Instruction::new(Opcode::PopStringLocal, Operand::Int(var_id))
    }

    pub fn push_long_local(var_id: i32) -> Self {
        Instruction::new(Opcode::PushLongLocal, Operand::Int(var_id))
    }

    pub fn pop_long_local(var_id: i32) -> Self {
        Instruction::new(Opcode::PopLongLocal, Operand::Int(var_id))
    }

    pub fn join_string(count: u32) -> Self {
        Instruction::new(Opcode::JoinString, Operand::StringCount(count))
    }
}

/// Compiled bytecode for a single script.
#[derive(Debug, Clone)]
pub struct CompiledScript {
    pub name: String,
    pub id: i32,
    pub trigger: String,
    pub source_path: String,
    /// Lookup key for fast engine dispatch (-1 if not applicable).
    pub lookup_key: i32,
    pub param_types: Vec<crate::types::Type>,
    pub instructions: Vec<Instruction>,
    pub local_table: crate::symbol::LocalTable,
    pub int_local_count: u16,
    pub string_local_count: u16,
    pub long_local_count: u16,
    pub int_arg_count: u16,
    pub string_arg_count: u16,
    pub long_arg_count: u16,
    pub switch_tables: Vec<Vec<(i32, usize)>>,
}

impl CompiledScript {
    pub fn new(name: String, id: i32) -> Self {
        Self {
            name,
            id,
            trigger: String::new(),
            source_path: String::new(),
            lookup_key: -1,
            param_types: Vec::new(),
            instructions: Vec::new(),
            local_table: crate::symbol::LocalTable::new(),
            int_local_count: 0,
            string_local_count: 0,
            long_local_count: 0,
            int_arg_count: 0,
            string_arg_count: 0,
            long_arg_count: 0,
            switch_tables: Vec::new(),
        }
    }

    pub fn push(&mut self, instruction: Instruction) {
        self.instructions.push(instruction);
    }

    pub fn len(&self) -> usize {
        self.instructions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Update a jump target at the given instruction index.
    pub fn patch_jump(&mut self, index: usize, target: usize) {
        self.instructions[index].operand = Operand::JumpTarget(target);
    }
}
