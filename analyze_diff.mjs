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

const refLengths = parseIdx('data/reference/script.idx');
const ourLengths = parseIdx('data/pack/server/script.idx');
const refScripts = parseDat('data/reference/script.dat', refLengths);
const ourScripts = parseDat('data/pack/server/script.dat', ourLengths);

let matches = 0, total = 0;
let sizeDiffCount = 0, contentDiffCount = 0;
const contentDiffs = [];

for (let i = 0; i < refLengths.length; i++) {
    const ref = refScripts[i];
    const our = ourScripts[i];
    if (!ref || ref.length === 0) { if (!our || our.length === 0) matches++; total++; continue; }
    if (!our || our.length === 0) { total++; continue; }
    total++;

    if (Buffer.compare(ref, our) === 0) { matches++; continue; }

    const refName = readString(ref, 0);
    const ourName = readString(our, 0);

    if (refName.str !== ourName.str) continue; // name mismatch - skip

    // Parse headers to find where instructions start
    // name\0 + path\0 + lookupKey(4) + paramTypeCount(1) + paramTypes(n) + lineTableLen(2) + lineTable
    const refPath = readString(ref, refName.end);
    const ourPath = readString(our, ourName.end);

    let refOff = refPath.end;
    let ourOff = ourPath.end;

    // lookupKey
    const refLookupKey = ref.readInt32BE(refOff); refOff += 4;
    const ourLookupKey = our.readInt32BE(ourOff); ourOff += 4;

    // paramTypeCount + types
    const refParamCount = ref[refOff++];
    const ourParamCount = our[ourOff++];
    refOff += refParamCount;
    ourOff += ourParamCount;

    // lineTableLen
    const refLineLen = ref.readUInt16BE(refOff); refOff += 2;
    const ourLineLen = our.readUInt16BE(ourOff); ourOff += 2;

    // Compare line tables
    let lineDiff = refLineLen !== ourLineLen;
    if (!lineDiff) {
        for (let j = 0; j < refLineLen * 8; j++) {
            if (ref[refOff + j] !== our[ourOff + j]) { lineDiff = true; break; }
        }
    }

    const refInstrStart = refOff + refLineLen * 8;
    const ourInstrStart = ourOff + ourLineLen * 8;

    // Compare from instruction start to end
    // Trailer is at the end: last 2 bytes = trailerLen
    const refTrailerLen = ref.readUInt16BE(ref.length - 2);
    const ourTrailerLen = our.readUInt16BE(our.length - 2);

    const refTrailerStart = ref.length - 2 - refTrailerLen - 12;
    const ourTrailerStart = our.length - 2 - ourTrailerLen - 12;

    // Compare instructions region
    const refInstrs = ref.subarray(refInstrStart, refTrailerStart);
    const ourInstrs = our.subarray(ourInstrStart, ourTrailerStart);

    let instrDiff = refInstrs.length !== ourInstrs.length;
    let instrDiffPos = -1;
    if (!instrDiff) {
        for (let j = 0; j < refInstrs.length; j++) {
            if (refInstrs[j] !== ourInstrs[j]) { instrDiff = true; instrDiffPos = j; break; }
        }
    }

    // Compare trailer
    const refTrailer = ref.subarray(refTrailerStart);
    const ourTrailer = our.subarray(ourTrailerStart);
    let trailerDiff = Buffer.compare(refTrailer, ourTrailer) !== 0;

    const diff = {
        id: i,
        name: refName.str,
        refLen: ref.length,
        ourLen: our.length,
        lookupKeyDiff: refLookupKey !== ourLookupKey,
        refLookupKey, ourLookupKey,
        lineDiff,
        refLineLen, ourLineLen,
        instrDiff,
        instrDiffPos,
        refInstrLen: refInstrs.length,
        ourInstrLen: ourInstrs.length,
        trailerDiff,
    };

    if (ref.length !== our.length) sizeDiffCount++;
    else contentDiffCount++;
    contentDiffs.push(diff);
}

console.log(`Total: ${total}, Matches: ${matches} (${(matches/total*100).toFixed(2)}%)`);
console.log(`Size diffs: ${sizeDiffCount}, Content-only diffs: ${contentDiffCount}`);

// Categorize diffs
let onlyLine = 0, onlyInstr = 0, onlyTrailer = 0, onlyLookup = 0, mixed = 0;
for (const d of contentDiffs) {
    const cats = [d.lineDiff, d.instrDiff, d.trailerDiff, d.lookupKeyDiff].filter(Boolean).length;
    if (cats === 1) {
        if (d.lineDiff) onlyLine++;
        else if (d.instrDiff) onlyInstr++;
        else if (d.trailerDiff) onlyTrailer++;
        else if (d.lookupKeyDiff) onlyLookup++;
    } else {
        mixed++;
    }
}
console.log(`\nDiff categories:`);
console.log(`  Line table only: ${onlyLine}`);
console.log(`  Instructions only: ${onlyInstr}`);
console.log(`  Trailer only: ${onlyTrailer}`);
console.log(`  LookupKey only: ${onlyLookup}`);
console.log(`  Mixed: ${mixed}`);

console.log(`\nFirst 20 detailed diffs:`);
for (const d of contentDiffs.slice(0, 20)) {
    const parts = [];
    if (d.lookupKeyDiff) parts.push(`lookup: ref=${d.refLookupKey} our=${d.ourLookupKey}`);
    if (d.lineDiff) parts.push(`lines: ref=${d.refLineLen} our=${d.ourLineLen}`);
    if (d.instrDiff) parts.push(`instrs: ref=${d.refInstrLen}b our=${d.ourInstrLen}b diffAt=${d.instrDiffPos}`);
    if (d.trailerDiff) parts.push(`trailer differs`);
    console.log(`  [${d.id}] ${d.name} (${d.refLen}b/${d.ourLen}b): ${parts.join(', ')}`);
}
