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

// is_large_operand from writer.rs
function isLargeOperand(opcode) {
    if (opcode > 100) return false;
    if ([21, 22, 23, 38, 39].includes(opcode)) return false;
    return true;
}

function disassemble(buf, start, end) {
    const instrs = [];
    let offset = start;
    while (offset < end) {
        const opcode = buf.readUInt16BE(offset);
        offset += 2;
        let operand;
        if (opcode > 100) {
            // Command - 1 byte operand
            operand = buf[offset++];
            instrs.push({ opcode, operand, isCmd: true });
        } else if (opcode === 3) {
            // PUSH_CONSTANT_STRING - null-terminated string
            let strEnd = offset;
            while (strEnd < end && buf[strEnd] !== 0) strEnd++;
            const str = buf.subarray(offset, strEnd).toString('utf8');
            offset = strEnd + 1;
            instrs.push({ opcode, str, isStr: true });
        } else if (isLargeOperand(opcode)) {
            operand = buf.readInt32BE(offset);
            offset += 4;
            instrs.push({ opcode, operand });
        } else {
            operand = buf[offset++];
            instrs.push({ opcode, operand });
        }
    }
    return instrs;
}

const refLengths = parseIdx('data/reference/script.idx');
const ourLengths = parseIdx('data/pack/server/script.idx');
const refScripts = parseDat('data/reference/script.dat', refLengths);
const ourScripts = parseDat('data/pack/server/script.dat', ourLengths);

// Find first N instruction-only diffs and show the differing instructions
let shown = 0;
const diffOperandValues = new Map(); // opcode -> count

for (let i = 0; i < refLengths.length && shown < 15; i++) {
    const ref = refScripts[i];
    const our = ourScripts[i];
    if (!ref || ref.length === 0 || !our || our.length === 0) continue;
    if (Buffer.compare(ref, our) === 0) continue;
    if (ref.length !== our.length) continue; // skip size diffs for now

    const refName = readString(ref, 0);
    const ourName = readString(our, 0);
    if (refName.str !== ourName.str) continue;

    const refPath = readString(ref, refName.end);
    const ourPath = readString(our, ourName.end);

    let refOff = refPath.end;
    let ourOff = ourPath.end;

    // Skip lookupKey, paramTypeCount, paramTypes, lineTable
    refOff += 4; ourOff += 4; // lookupKey
    const rpc = ref[refOff++]; const opc = our[ourOff++];
    refOff += rpc; ourOff += opc;
    const rll = ref.readUInt16BE(refOff); refOff += 2;
    const oll = our.readUInt16BE(ourOff); ourOff += 2;
    refOff += rll * 8;
    ourOff += oll * 8;

    // Trailer
    const refTrailerLen = ref.readUInt16BE(ref.length - 2);
    const ourTrailerLen = our.readUInt16BE(our.length - 2);
    const refTrailerStart = ref.length - 2 - refTrailerLen - 12;
    const ourTrailerStart = our.length - 2 - ourTrailerLen - 12;

    const refInstrs = disassemble(ref, refOff, refTrailerStart);
    const ourInstrs = disassemble(our, ourOff, ourTrailerStart);

    if (refInstrs.length !== ourInstrs.length) continue;

    const diffs = [];
    for (let j = 0; j < refInstrs.length; j++) {
        const r = refInstrs[j], o = ourInstrs[j];
        if (r.opcode !== o.opcode || r.operand !== o.operand || r.str !== o.str) {
            diffs.push({ idx: j, ref: r, our: o });
            const key = r.opcode;
            diffOperandValues.set(key, (diffOperandValues.get(key) || 0) + 1);
        }
    }

    if (diffs.length > 0) {
        shown++;
        console.log(`\n[${i}] ${refName.str} (${refInstrs.length} instrs, ${diffs.length} diffs):`);
        for (const d of diffs.slice(0, 5)) {
            const r = d.ref, o = d.our;
            if (r.isStr || o.isStr) {
                console.log(`  #${d.idx}: ref(op=${r.opcode},"${r.str}") our(op=${o.opcode},"${o.str}")`);
            } else {
                console.log(`  #${d.idx}: ref(op=${r.opcode},${r.operand}) our(op=${o.opcode},${o.operand})`);
            }
        }
    }
}

console.log('\n\nOpcode frequency in diffs:');
const sorted = [...diffOperandValues.entries()].sort((a,b) => b[1] - a[1]);
for (const [op, count] of sorted.slice(0, 20)) {
    console.log(`  opcode ${op}: ${count} diffs`);
}
