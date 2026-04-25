import fs from 'fs';

// Parse the reference script.dat to extract what opcodes are used for what commands
// by cross-referencing with the source .rs2 files

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

const refLengths = parseIdx('data/reference/script.idx');
const ourLengths = parseIdx('data/pack/server/script.idx');
const refScripts = parseDat('data/reference/script.dat', refLengths);
const ourScripts = parseDat('data/pack/server/script.dat', ourLengths);

// For each matching-size script that differs, find the first differing opcode
// and report what ref opcode maps to what our opcode
const opcodeMapping = new Map(); // ref_opcode -> our_opcode -> count

function isLargeOperand(opcode) {
    if (opcode > 100) return false;
    if ([21, 22, 23, 38, 39].includes(opcode)) return false;
    return true;
}

function readString(buf, offset) {
    let end = offset;
    while (end < buf.length && buf[end] !== 0) end++;
    return { str: buf.subarray(offset, end).toString('utf8'), end: end + 1 };
}

function skipHeader(buf) {
    const name = readString(buf, 0);
    const path = readString(buf, name.end);
    let off = path.end;
    off += 4; // lookupKey
    const pc = buf[off++]; off += pc; // paramTypes
    const ll = buf.readUInt16BE(off); off += 2; off += ll * 8; // lineTable
    return off;
}

function getInstrOpcodes(buf, start) {
    const trailerLen = buf.readUInt16BE(buf.length - 2);
    const trailerStart = buf.length - 2 - trailerLen - 12;
    const opcodes = [];
    let offset = start;
    while (offset < trailerStart) {
        const opcode = buf.readUInt16BE(offset);
        offset += 2;
        opcodes.push(opcode);
        if (opcode > 100) {
            offset += 1; // cmd: 1 byte
        } else if (opcode === 3) {
            while (offset < trailerStart && buf[offset] !== 0) offset++;
            offset++; // null
        } else if (isLargeOperand(opcode)) {
            offset += 4;
        } else {
            offset += 1;
        }
    }
    return opcodes;
}

let count = 0;
for (let i = 0; i < refLengths.length && count < 500; i++) {
    const ref = refScripts[i];
    const our = ourScripts[i];
    if (!ref || ref.length === 0 || !our || our.length === 0) continue;
    if (ref.length !== our.length) continue;
    if (Buffer.compare(ref, our) === 0) continue;

    const refStart = skipHeader(ref);
    const ourStart = skipHeader(our);
    const refOps = getInstrOpcodes(ref, refStart);
    const ourOps = getInstrOpcodes(our, ourStart);

    if (refOps.length !== ourOps.length) continue;
    count++;

    for (let j = 0; j < refOps.length; j++) {
        if (refOps[j] !== ourOps[j]) {
            const key = `${refOps[j]}->${ourOps[j]}`;
            opcodeMapping.set(key, (opcodeMapping.get(key) || 0) + 1);
        }
    }
}

console.log('Opcode remapping (ref -> ours): count');
const sorted = [...opcodeMapping.entries()].sort((a,b) => b[1] - a[1]);
for (const [key, cnt] of sorted) {
    const [refOp, ourOp] = key.split('->').map(Number);
    console.log(`  ref ${refOp} -> our ${ourOp} (diff=${ourOp-refOp}): ${cnt} occurrences`);
}
