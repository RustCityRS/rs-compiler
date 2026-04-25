import fs from 'fs';

function parseIdx(path) {
    const buf = fs.readFileSync(path);
    const count = buf.readUInt32BE(0);
    const lengths = [];
    for (let i = 0; i < count; i++) lengths.push(buf.readUInt32BE(4 + i * 4));
    return lengths;
}
function parseDat(path, lengths) {
    const buf = fs.readFileSync(path);
    let offset = 8;
    const scripts = [];
    for (const len of lengths) { scripts.push(buf.subarray(offset, offset + len)); offset += len; }
    return scripts;
}
function readString(buf, offset) {
    let end = offset;
    while (end < buf.length && buf[end] !== 0) end++;
    return { str: buf.subarray(offset, end).toString('utf8'), end: end + 1 };
}
function isLargeOperand(opcode) {
    if (opcode > 100) return false;
    if ([21, 22, 23, 38, 39].includes(opcode)) return false;
    return true;
}

// Load command names
const cmdSym = fs.readFileSync('2004scape/Engine-TS/data/symbols/commands.sym', 'utf8');
const cmdNames = {};
for (const line of cmdSym.split('\n')) {
    const parts = line.split('\t');
    if (parts.length >= 2) cmdNames[parseInt(parts[0])] = parts[1];
}

function disassembleVerbose(buf, start, end, cmdNames) {
    const instrs = [];
    let offset = start;
    while (offset < end) {
        const pos = offset - start;
        const opcode = buf.readUInt16BE(offset);
        offset += 2;
        if (opcode > 100) {
            const operand = buf[offset++];
            const name = cmdNames[opcode] || `cmd_${opcode}`;
            instrs.push({ pos, opcode, operand, name, isCmd: true });
        } else if (opcode === 3) {
            let strEnd = offset;
            while (strEnd < end && buf[strEnd] !== 0) strEnd++;
            const str = buf.subarray(offset, strEnd).toString('utf8');
            offset = strEnd + 1;
            instrs.push({ pos, opcode, str, name: 'PUSH_STR' });
        } else if (isLargeOperand(opcode)) {
            const operand = buf.readInt32BE(offset);
            offset += 4;
            const names = {0:'PUSH_INT',1:'PUSH_VARP',2:'POP_VARP',4:'PUSH_VARN',5:'POP_VARN',6:'BRANCH',7:'BRANCH_NOT',8:'BRANCH_EQ',9:'BRANCH_LT',10:'BRANCH_GT',11:'PUSH_VARS',12:'POP_VARS',25:'PUSH_VARBIT',27:'POP_VARBIT',31:'BRANCH_LTE',32:'BRANCH_GTE',33:'PUSH_INT_LOCAL',34:'POP_INT_LOCAL',35:'PUSH_STR_LOCAL',36:'POP_STR_LOCAL',44:'DEF_ARRAY',45:'PUSH_ARRAY',46:'POP_ARRAY'};
            instrs.push({ pos, opcode, operand, name: names[opcode] || `op_${opcode}` });
        } else {
            const operand = buf[offset++];
            const names = {21:'RETURN',22:'GOSUB',23:'JUMP',24:'SWITCH',37:'JOIN_STRING',38:'POP_INT_DISCARD',39:'POP_STR_DISCARD',40:'GOSUB_PARAMS',41:'JUMP_PARAMS'};
            instrs.push({ pos, opcode, operand, name: names[opcode] || `op_${opcode}` });
        }
    }
    return instrs;
}

const targetId = parseInt(process.argv[2] || '3');

const refLengths = parseIdx('data/reference/script.idx');
const ourLengths = parseIdx('data/pack/server/script.idx');
const refScripts = parseDat('data/reference/script.dat', refLengths);
const ourScripts = parseDat('data/pack/server/script.dat', ourLengths);

const ref = refScripts[targetId];
const our = ourScripts[targetId];

for (const [label, buf] of [['REF', ref], ['OUR', our]]) {
    if (!buf || buf.length === 0) { console.log(`${label}: empty`); continue; }
    const name = readString(buf, 0);
    const path = readString(buf, name.end);
    let off = path.end;
    const lookupKey = buf.readInt32BE(off); off += 4;
    const paramCount = buf[off++]; off += paramCount;
    const lineLen = buf.readUInt16BE(off); off += 2;
    off += lineLen * 8;

    const trailerLen = buf.readUInt16BE(buf.length - 2);
    const trailerStart = buf.length - 2 - trailerLen - 12;

    console.log(`\n${label}: ${name.str} (${buf.length}b, ${lineLen} line entries)`);
    console.log(`  Path: ${path.str}`);
    const instrs = disassembleVerbose(buf, off, trailerStart, cmdNames);
    for (const instr of instrs) {
        if (instr.str !== undefined) {
            console.log(`  @${instr.pos}: ${instr.name} "${instr.str.substring(0, 60)}"`);
        } else {
            console.log(`  @${instr.pos}: ${instr.name}(${instr.opcode}) = ${instr.operand}`);
        }
    }
}
