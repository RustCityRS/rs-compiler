import fs from 'fs';
import { execSync } from 'child_process';

const id = parseInt(process.argv[2] || '1923');
const output = execSync(`node trace_script.mjs ${id}`, { encoding: 'utf8' });
const lines = output.split('\n');

const refInstrs = [];
const ourInstrs = [];
let current = null;
for (const line of lines) {
    if (line.startsWith('REF:')) { current = 'ref'; continue; }
    if (line.startsWith('OUR:')) { current = 'our'; continue; }
    if (line.startsWith('  Path:')) continue;
    if (line.startsWith('  @') && current === 'ref') refInstrs.push(line.trim());
    if (line.startsWith('  @') && current === 'our') ourInstrs.push(line.trim());
}

// Strip position prefix for comparison
const strip = s => s.replace(/^@\d+: /, '');

let ri = 0, oi = 0;
while (ri < refInstrs.length || oi < ourInstrs.length) {
    const r = ri < refInstrs.length ? strip(refInstrs[ri]) : '<end>';
    const o = oi < ourInstrs.length ? strip(ourInstrs[oi]) : '<end>';
    if (r === o) {
        ri++; oi++;
    } else {
        // Find if ref has extra or our has extra
        // Check if skipping ref matches
        if (ri + 1 < refInstrs.length && strip(refInstrs[ri + 1]) === o) {
            console.log(`REF EXTRA [${ri}]: ${refInstrs[ri]}`);
            ri++;
        } else if (oi + 1 < ourInstrs.length && strip(ourInstrs[oi + 1]) === r) {
            console.log(`OUR EXTRA [${oi}]: ${ourInstrs[oi]}`);
            oi++;
        } else {
            console.log(`DIFF [ref ${ri}, our ${oi}]:`);
            console.log(`  REF: ${refInstrs[ri] || '<end>'}`);
            console.log(`  OUR: ${ourInstrs[oi] || '<end>'}`);
            ri++; oi++;
        }
    }
}
