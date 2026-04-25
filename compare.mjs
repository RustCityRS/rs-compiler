import fs from 'fs';

// Parse script.idx to get individual script lengths
function parseIdx(idxPath) {
    const buf = fs.readFileSync(idxPath);
    const count = buf.readUInt32BE(0);
    const lengths = [];
    for (let i = 0; i < count; i++) {
        lengths.push(buf.readUInt32BE(4 + i * 4));
    }
    return lengths;
}

// Parse script.dat to get individual script bytes
function parseDat(datPath, lengths) {
    const buf = fs.readFileSync(datPath);
    // Header: count (4 bytes) + version (4 bytes) = 8 bytes
    let offset = 8;
    const scripts = [];
    for (const len of lengths) {
        scripts.push(buf.subarray(offset, offset + len));
        offset += len;
    }
    return scripts;
}

// Extract script name from encoded bytes (null-terminated at start)
function getScriptName(bytes) {
    let end = 0;
    while (end < bytes.length && bytes[end] !== 0) end++;
    return bytes.subarray(0, end).toString('utf8');
}

const refLengths = parseIdx('data/reference/script.idx');
const ourLengths = parseIdx('data/pack/server/script.idx');

const refScripts = parseDat('data/reference/script.dat', refLengths);
const ourScripts = parseDat('data/pack/server/script.dat', ourLengths);

console.log(`Reference: ${refLengths.length} scripts`);
console.log(`Ours:      ${ourLengths.length} scripts`);

const total = Math.max(refLengths.length, ourLengths.length);
let matches = 0;
let mismatches = [];
let missing = 0;

for (let i = 0; i < total; i++) {
    const ref = refScripts[i];
    const our = ourScripts[i];

    if (!ref || !our) {
        missing++;
        if (!our && ref) {
            const name = getScriptName(ref);
            mismatches.push({ id: i, name, reason: 'missing in ours' });
        } else if (!ref && our) {
            const name = getScriptName(our);
            mismatches.push({ id: i, name, reason: 'extra in ours' });
        }
        continue;
    }

    if (Buffer.compare(ref, our) === 0) {
        matches++;
    } else {
        const refName = getScriptName(ref);
        const ourName = getScriptName(our);

        // Find first byte difference
        let diffPos = -1;
        for (let j = 0; j < Math.max(ref.length, our.length); j++) {
            if ((ref[j] ?? -1) !== (our[j] ?? -1)) {
                diffPos = j;
                break;
            }
        }

        // Show hex dump around first diff for first few mismatches
        let hexDump = '';
        if (refName === ourName && mismatches.length < 5) {
            const start = Math.max(0, diffPos - 4);
            const end = Math.min(Math.max(ref.length, our.length), diffPos + 20);
            const refHex = ref.subarray(start, Math.min(end, ref.length));
            const ourHex = our.subarray(start, Math.min(end, our.length));
            hexDump = `\n      ref[${start}..]: ${Buffer.from(refHex).toString('hex').match(/../g).join(' ')}\n      our[${start}..]: ${Buffer.from(ourHex).toString('hex').match(/../g).join(' ')}`;
        }

        mismatches.push({
            id: i,
            name: refName,
            ourName,
            refLen: ref.length,
            ourLen: our.length,
            diffPos,
            hexDump,
            reason: refName !== ourName ? 'name mismatch' : 'bytecode differs'
        });
    }
}

console.log(`\nMatches:    ${matches}/${refLengths.length} (${(matches/refLengths.length*100).toFixed(2)}%)`);
console.log(`Mismatches: ${mismatches.length}`);
console.log(`Missing:    ${missing}`);

if (mismatches.length > 0) {
    console.log(`\nFirst 50 mismatches:`);
    for (const m of mismatches.slice(0, 50)) {
        if (m.reason === 'name mismatch') {
            console.log(`  [${m.id}] NAME MISMATCH: ref="${m.name}" ours="${m.ourName}" (ref ${m.refLen}b, ours ${m.ourLen}b)`);
        } else if (m.reason === 'bytecode differs') {
            console.log(`  [${m.id}] ${m.name}: ref ${m.refLen}b vs ours ${m.ourLen}b, first diff at byte ${m.diffPos}${m.hexDump || ''}`);
        } else {
            console.log(`  [${m.id}] ${m.name}: ${m.reason}`);
        }
    }
}
