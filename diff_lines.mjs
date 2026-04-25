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

const id = parseInt(process.argv[2] || '1686');
const refLengths = parseIdx('data/reference/script.idx');
const ourLengths = parseIdx('data/pack/server/script.idx');
const refScripts = parseDat('data/reference/script.dat', refLengths);
const ourScripts = parseDat('data/pack/server/script.dat', ourLengths);

for (const [label, buf] of [['REF', refScripts[id]], ['OUR', ourScripts[id]]]) {
    const name = readString(buf, 0);
    const path = readString(buf, name.end);
    let off = path.end;
    off += 4; // lookupKey
    const paramCount = buf[off++]; off += paramCount;
    const lineLen = buf.readUInt16BE(off); off += 2;

    console.log(`${label}: ${name.str} (${lineLen} line entries)`);
    for (let i = 0; i < lineLen; i++) {
        const pc = buf.readInt32BE(off + i * 8);
        const line = buf.readInt32BE(off + i * 8 + 4);
        console.log(`  pc=${pc} line=${line}`);
    }
}
