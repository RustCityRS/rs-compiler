use crate::opcodes;

/// RS2 opcodes matching the actual wire format.
/// These values correspond to the opcodes used by the RuneScape 2 script engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Opcode {
    PushConstantInt = opcodes::PUSH_CONSTANT_INT,
    PushVarp = opcodes::PUSH_VARP,
    PopVarp = opcodes::POP_VARP,
    PushConstantString = opcodes::PUSH_CONSTANT_STRING,
    PushVarn = opcodes::PUSH_VARN,
    PopVarn = opcodes::POP_VARN,
    Branch = opcodes::BRANCH,
    BranchNot = opcodes::BRANCH_NOT,
    BranchEquals = opcodes::BRANCH_EQUALS,
    BranchLessThan = opcodes::BRANCH_LESS_THAN,
    BranchGreaterThan = opcodes::BRANCH_GREATER_THAN,
    PushVars = opcodes::PUSH_VARS,
    PopVars = opcodes::POP_VARS,
    Return = opcodes::RETURN,
    Gosub = opcodes::GOSUB,
    Jump = opcodes::JUMP,
    Switch = opcodes::SWITCH,
    PushVarbit = opcodes::PUSH_VARBIT,
    PopVarbit = opcodes::POP_VARBIT,
    BranchLessThanOrEquals = opcodes::BRANCH_LESS_THAN_OR_EQUALS,
    BranchGreaterThanOrEquals = opcodes::BRANCH_GREATER_THAN_OR_EQUALS,
    PushIntLocal = opcodes::PUSH_INT_LOCAL,
    PopIntLocal = opcodes::POP_INT_LOCAL,
    PushStringLocal = opcodes::PUSH_STRING_LOCAL,
    PopStringLocal = opcodes::POP_STRING_LOCAL,
    JoinString = opcodes::JOIN_STRING,
    PopIntDiscard = opcodes::POP_INT_DISCARD,
    PopStringDiscard = opcodes::POP_STRING_DISCARD,
    GosubWithParams = opcodes::GOSUB_WITH_PARAMS,
    JumpWithParams = opcodes::JUMP_WITH_PARAMS,
    PushVarcInt = opcodes::PUSH_VARC_INT,
    PopVarcInt = opcodes::POP_VARC_INT,
    DefineArray = opcodes::DEFINE_ARRAY,
    PushArrayInt = opcodes::PUSH_ARRAY_INT,
    PopArrayInt = opcodes::POP_ARRAY_INT,
    PushVarcString = opcodes::PUSH_VARC_STRING,
    PopVarcString = opcodes::POP_VARC_STRING,
    PushConstantLong = opcodes::PUSH_CONSTANT_LONG,
    PushLongLocal = opcodes::PUSH_LONG_LOCAL,
    PopLongLocal = opcodes::POP_LONG_LOCAL,
    PopLongDiscard = opcodes::POP_LONG_DISCARD,
    // Long branch operations
    LongBranchNot = opcodes::LONG_BRANCH_NOT,
    LongBranchEquals = opcodes::LONG_BRANCH_EQUALS,
    LongBranchLessThan = opcodes::LONG_BRANCH_LESS_THAN,
    LongBranchGreaterThan = opcodes::LONG_BRANCH_GREATER_THAN,
    LongBranchLessThanOrEquals = opcodes::LONG_BRANCH_LESS_THAN_OR_EQUALS,
    LongBranchGreaterThanOrEquals = opcodes::LONG_BRANCH_GREATER_THAN_OR_EQUALS,
    // Server-only vars, in a range the client leaves unused
    PushVarClan = opcodes::PUSH_VARCLAN,
    PopVarClan = opcodes::POP_VARCLAN,
    PushVarObj = opcodes::PUSH_VAROBJ,
    PopVarObj = opcodes::POP_VAROBJ,
    PushVarnBit = opcodes::PUSH_VARNBIT,
    PopVarnBit = opcodes::POP_VARNBIT,
    PushVarCon = opcodes::PUSH_VARCON,
    PopVarCon = opcodes::POP_VARCON,
    PopClanSettingInt = opcodes::POP_CLANSETTING_INT,
    PopClanSettingVarbit = opcodes::POP_CLANSETTING_VARBIT,
    PopClanSettingLong = opcodes::POP_CLANSETTING_LONG,
    PopClanSettingString = opcodes::POP_CLANSETTING_STRING,
    // Object/string branch operations
    ObjBranchEquals = opcodes::OBJ_BRANCH_EQUALS,
    ObjBranchNot = opcodes::OBJ_BRANCH_NOT,
    PushVarConBit = opcodes::PUSH_VARCONBIT,
    PopVarConBit = opcodes::POP_VARCONBIT,
    PushClanSettingInt = opcodes::PUSH_CLANSETTING_INT,
    PushClanSettingVarbit = opcodes::PUSH_CLANSETTING_VARBIT,
    PushClanSettingLong = opcodes::PUSH_CLANSETTING_LONG,
    PushClanSettingString = opcodes::PUSH_CLANSETTING_STRING,
    // Arithmetic operations (4600+ range — engine commands)
    Add = opcodes::ADD,
    Sub = opcodes::SUB,
    Multiply = opcodes::MULTIPLY,
    Divide = opcodes::DIVIDE,
    Random = opcodes::RANDOM,
    RandomInc = opcodes::RANDOMINC,
    Interpolate = opcodes::INTERPOLATE,
    AddPercent = opcodes::ADDPERCENT,
    SetBit = opcodes::SETBIT,
    ClearBit = opcodes::CLEARBIT,
    TestBit = opcodes::TESTBIT,
    Modulo = opcodes::MODULO,
    Pow = opcodes::POW,
    InvPow = opcodes::INVPOW,
    And = opcodes::AND,
    Or = opcodes::OR,
    Min = opcodes::MIN,
    Max = opcodes::MAX,
    Scale = opcodes::SCALE,
    BitCount = opcodes::BITCOUNT,
    ToggleBit = opcodes::TOGGLEBIT,
    SetBitRange = opcodes::SETBIT_RANGE,
    ClearBitRange = opcodes::CLEARBIT_RANGE,
    GetBitRange = opcodes::GETBIT_RANGE,
    SetBitRangeToVal = opcodes::SETBIT_RANGE_TOINT,
    SinDeg = opcodes::SIN_DEG,
    CosDeg = opcodes::COS_DEG,
    Atan2Deg = opcodes::ATAN2_DEG,
    Abs = opcodes::ABS,
    // String operations (4200+ range)
    // Command is used for engine commands - opcode is looked up dynamically
    Command = 65535,
    Raw = 65533,
    BranchIfTrue = 65532,
    BranchIfFalse = 65531,
    // Line number metadata (not a real opcode, used for debugging)
    LineNumber = 65534,
}

impl Opcode {
    /// The opcode that reads a game var of `category`.
    pub fn push_var(category: &str) -> Self {
        match category {
            "varn" => Self::PushVarn,
            "vars" => Self::PushVars,
            "varpbit" => Self::PushVarbit,
            "varc" => Self::PushVarcInt,
            "varcstr" => Self::PushVarcString,
            "varclan" => Self::PushVarClan,
            "varobj" => Self::PushVarObj,
            "varnbit" => Self::PushVarnBit,
            "varcon" => Self::PushVarCon,
            "varconbit" => Self::PushVarConBit,
            "clansetting_int" => Self::PushClanSettingInt,
            "clansetting_varbit" => Self::PushClanSettingVarbit,
            "clansetting_long" => Self::PushClanSettingLong,
            "clansetting_string" => Self::PushClanSettingString,
            _ => Self::PushVarp,
        }
    }

    /// The opcode that writes a game var of `category`.
    pub fn pop_var(category: &str) -> Self {
        match category {
            "varn" => Self::PopVarn,
            "vars" => Self::PopVars,
            "varpbit" => Self::PopVarbit,
            "varc" => Self::PopVarcInt,
            "varcstr" => Self::PopVarcString,
            "varclan" => Self::PopVarClan,
            "varobj" => Self::PopVarObj,
            "varnbit" => Self::PopVarnBit,
            "varcon" => Self::PopVarCon,
            "varconbit" => Self::PopVarConBit,
            "clansetting_int" => Self::PopClanSettingInt,
            "clansetting_varbit" => Self::PopClanSettingVarbit,
            "clansetting_long" => Self::PopClanSettingLong,
            "clansetting_string" => Self::PopClanSettingString,
            _ => Self::PopVarp,
        }
    }

    pub fn core(name: &str) -> Option<Self> {
        match name {
            "PushConstantInt" => Some(Self::PushConstantInt),
            "PushConstantString" => Some(Self::PushConstantString),
            "PushConstantLong" => Some(Self::PushConstantLong),
            "PushVarp" => Some(Self::PushVarp),
            "PopVarp" => Some(Self::PopVarp),
            "PushVarn" => Some(Self::PushVarn),
            "PopVarn" => Some(Self::PopVarn),
            "PushVars" => Some(Self::PushVars),
            "PopVars" => Some(Self::PopVars),
            "PushVarbit" => Some(Self::PushVarbit),
            "PopVarbit" => Some(Self::PopVarbit),
            "PushVarcInt" => Some(Self::PushVarcInt),
            "PopVarcInt" => Some(Self::PopVarcInt),
            "PushVarcString" => Some(Self::PushVarcString),
            "PopVarcString" => Some(Self::PopVarcString),
            "PushClanSettingInt" => Some(Self::PushClanSettingInt),
            "PushClanSettingVarbit" => Some(Self::PushClanSettingVarbit),
            "PushClanSettingLong" => Some(Self::PushClanSettingLong),
            "PushClanSettingString" => Some(Self::PushClanSettingString),
            "Branch" => Some(Self::Branch),
            "BranchNot" => Some(Self::BranchNot),
            "BranchEquals" => Some(Self::BranchEquals),
            "BranchLessThan" => Some(Self::BranchLessThan),
            "BranchGreaterThan" => Some(Self::BranchGreaterThan),
            "BranchLessThanOrEquals" => Some(Self::BranchLessThanOrEquals),
            "BranchGreaterThanOrEquals" => Some(Self::BranchGreaterThanOrEquals),
            "LongBranchNot" => Some(Self::LongBranchNot),
            "LongBranchEquals" => Some(Self::LongBranchEquals),
            "LongBranchLessThan" => Some(Self::LongBranchLessThan),
            "LongBranchGreaterThan" => Some(Self::LongBranchGreaterThan),
            "LongBranchLessThanOrEquals" => Some(Self::LongBranchLessThanOrEquals),
            "LongBranchGreaterThanOrEquals" => Some(Self::LongBranchGreaterThanOrEquals),
            "BranchIfTrue" => Some(Self::BranchIfTrue),
            "BranchIfFalse" => Some(Self::BranchIfFalse),
            "ObjBranchEquals" => Some(Self::ObjBranchEquals),
            "ObjBranchNot" => Some(Self::ObjBranchNot),
            "Return" => Some(Self::Return),
            "Gosub" => Some(Self::Gosub),
            "GosubWithParams" => Some(Self::GosubWithParams),
            "Jump" => Some(Self::Jump),
            "JumpWithParams" => Some(Self::JumpWithParams),
            "Switch" => Some(Self::Switch),
            "PushIntLocal" => Some(Self::PushIntLocal),
            "PopIntLocal" => Some(Self::PopIntLocal),
            "PushStringLocal" => Some(Self::PushStringLocal),
            "PopStringLocal" => Some(Self::PopStringLocal),
            "PushLongLocal" => Some(Self::PushLongLocal),
            "PopLongLocal" => Some(Self::PopLongLocal),
            "PopIntDiscard" => Some(Self::PopIntDiscard),
            "PopStringDiscard" => Some(Self::PopStringDiscard),
            "PopLongDiscard" => Some(Self::PopLongDiscard),
            "JoinString" => Some(Self::JoinString),
            "DefineArray" => Some(Self::DefineArray),
            "PushArrayInt" => Some(Self::PushArrayInt),
            "PopArrayInt" => Some(Self::PopArrayInt),
            _ => None,
        }
    }
}

/// A single instruction in the compiled bytecode.
#[derive(Debug, Clone)]
pub struct Instruction {
    pub opcode: Opcode,
    pub operand: Operand,
}

/// The operand (data) associated with an instruction.
///
/// Kept as small as possible (each script holds a `Vec<Instruction>`, and the
/// corpus runs to ~half a million instructions): the two heap-backed payloads —
/// string literals and switch tables — live in side pools on the owning
/// `CompiledScript`, and the operand stores only a `u32` index into them. That
/// keeps every variant ≤ 8 B, so `Operand` is 16 B and `Instruction` is 24 B
/// (vs 24/32 when the payloads were inline `Box<str>`/`Box<[_]>`).
#[derive(Debug, Clone)]
pub enum Operand {
    None,
    Int(i32),
    Long(i64),
    Str(u32),
    JumpTarget(usize),
    SwitchTable(u32),
    StringCount(u32),
    ArrayDef(i32, u8),
}

impl Instruction {
    pub fn new(opcode: Opcode, operand: Operand) -> Self {
        Instruction { opcode, operand }
    }

    pub fn simple(opcode: Opcode) -> Self {
        Instruction {
            opcode,
            operand: Operand::None,
        }
    }

    pub fn push_int(value: i32) -> Self {
        Instruction::new(Opcode::PushConstantInt, Operand::Int(value))
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
    pub strings: Vec<Box<str>>,
    pub switch_tables: Vec<Box<[(i32, usize)]>>,
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
            strings: Vec::new(),
            switch_tables: Vec::new(),
        }
    }

    pub fn push(&mut self, instruction: Instruction) {
        self.instructions.push(instruction);
    }

    /// Add a string payload to the pool and return its `Operand::Str` index.
    pub fn intern_str(&mut self, value: String) -> u32 {
        let idx = self.strings.len() as u32;
        self.strings.push(value.into_boxed_str());
        idx
    }

    /// Emit a `PushConstantString` whose literal is interned into the pool.
    pub fn push_string(&mut self, value: String) {
        let idx = self.intern_str(value);
        self.instructions.push(Instruction::new(
            Opcode::PushConstantString,
            Operand::Str(idx),
        ));
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

    /// Release the spare capacity the growable codegen buffers left behind.
    /// Compiled scripts are immutable after codegen but are held in memory for
    /// the whole pointer-check + write phase, so reclaiming each `Vec`'s doubling
    /// slack (often 25-50%) meaningfully lowers the resident footprint.
    pub fn shrink_to_fit(&mut self) {
        self.instructions.shrink_to_fit();
        self.param_types.shrink_to_fit();
        self.local_table.shrink_to_fit();
        self.strings.shrink_to_fit();
        self.switch_tables.shrink_to_fit();
    }
}
