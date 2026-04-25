#!/usr/bin/env python3
"""Decode and compare individual script binary blobs from script.dat/script.idx.

Usage:
    python3 decode_script.py <dat> <idx> <script_id> [--compare <dat2> <idx2>]
"""

import struct
import sys


def read_idx(path):
    with open(path, "rb") as f:
        data = f.read()
    count = struct.unpack(">I", data[0:4])[0]
    entries = []
    offset = 8
    for i in range(count):
        length = struct.unpack(">I", data[4 + i * 4 : 8 + i * 4])[0]
        entries.append((offset, length))
        offset += length
    return entries


def read_dat(path):
    with open(path, "rb") as f:
        return f.read()


SMALL_OPCODES = {21, 22, 23, 38, 39}  # return, gosub, jump, pop_int_discard, pop_string_discard


def is_large(opcode):
    if opcode > 100:
        return False
    return opcode not in SMALL_OPCODES


OPCODE_NAMES = {
    0: "PUSH_CONSTANT_INT",
    1: "PUSH_VARP",
    2: "POP_VARP",
    3: "PUSH_CONSTANT_STRING",
    6: "BRANCH",
    7: "BRANCH_NOT",
    8: "BRANCH_EQUALS",
    9: "BRANCH_LESS_THAN",
    10: "BRANCH_GREATER_THAN",
    21: "RETURN",
    22: "GOSUB",
    23: "JUMP",
    24: "SWITCH",
    25: "PUSH_VARN",
    26: "POP_VARN",
    27: "PUSH_VARS",
    28: "POP_VARS",
    31: "BRANCH_LESS_THAN_OR_EQUALS",
    32: "BRANCH_GREATER_THAN_OR_EQUALS",
    33: "PUSH_INT_LOCAL",
    34: "POP_INT_LOCAL",
    35: "PUSH_STRING_LOCAL",
    36: "POP_STRING_LOCAL",
    37: "JOIN_STRING",
    38: "POP_INT_DISCARD",
    39: "POP_STRING_DISCARD",
    40: "PUSH_CONSTANT_INT",  # DEFINE_ARRAY
    41: "PUSH_ARRAY_INT",
    42: "POP_ARRAY_INT",
    68: "BRANCH_NOT_S",
    69: "BRANCH_EQUALS_S",
    70: "BRANCH_LESS_THAN_S",
    71: "BRANCH_GREATER_THAN_S",
    72: "BRANCH_LESS_THAN_OR_EQUALS_S",
    73: "BRANCH_GREATER_THAN_OR_EQUALS_S",
    86: "JUMP_WITH_PARAMS",
    87: "GOSUB_WITH_PARAMS",
}


def decode_script(data, offset, length):
    """Decode a script blob into structured components."""
    blob = data[offset:offset + length]
    pos = 0
    result = {}

    # Script name (null-terminated)
    name_end = blob.index(b"\x00", pos)
    result["name"] = blob[pos:name_end].decode("latin-1")
    pos = name_end + 1

    # Source file path (null-terminated)
    path_end = blob.index(b"\x00", pos)
    result["source_path"] = blob[pos:path_end].decode("latin-1")
    pos = path_end + 1

    # Lookup key (i32)
    result["lookup_key"] = struct.unpack(">i", blob[pos:pos+4])[0]
    pos += 4

    # Param type count + types
    param_count = blob[pos]
    pos += 1
    result["param_types"] = list(blob[pos:pos+param_count])
    pos += param_count

    # Line number table
    line_count = struct.unpack(">H", blob[pos:pos+2])[0]
    pos += 2
    line_table = []
    for _ in range(line_count):
        pc = struct.unpack(">i", blob[pos:pos+4])[0]
        line = struct.unpack(">i", blob[pos+4:pos+8])[0]
        line_table.append((pc, line))
        pos += 8
    result["line_table"] = line_table

    # Read trailer from end
    trailer_var_len = struct.unpack(">H", blob[-2:])[0]
    trailer_start = len(blob) - 2 - trailer_var_len - 12

    # Trailer: instr_count(4) + intLocals(2) + strLocals(2) + intArgs(2) + strArgs(2) = 12
    t = trailer_start
    instr_count = struct.unpack(">i", blob[t:t+4])[0]
    t += 4
    int_locals = struct.unpack(">H", blob[t:t+2])[0]
    t += 2
    str_locals = struct.unpack(">H", blob[t:t+2])[0]
    t += 2
    int_args = struct.unpack(">H", blob[t:t+2])[0]
    t += 2
    str_args = struct.unpack(">H", blob[t:t+2])[0]
    t += 2

    result["instr_count"] = instr_count
    result["int_locals"] = int_locals
    result["str_locals"] = str_locals
    result["int_args"] = int_args
    result["str_args"] = str_args

    # Switch tables
    switch_count = blob[t]
    t += 1
    switches = []
    for _ in range(switch_count):
        case_count = struct.unpack(">H", blob[t:t+2])[0]
        t += 2
        cases = []
        for _ in range(case_count):
            key = struct.unpack(">i", blob[t:t+4])[0]
            off = struct.unpack(">i", blob[t+4:t+8])[0]
            cases.append((key, off))
            t += 8
        switches.append(cases)
    result["switches"] = switches

    # Decode instructions
    instructions = []
    ipos = pos
    for _ in range(instr_count):
        if ipos >= trailer_start:
            break
        opcode = struct.unpack(">H", blob[ipos:ipos+2])[0]
        ipos += 2

        if opcode == 3:  # PUSH_CONSTANT_STRING
            str_end = blob.index(b"\x00", ipos)
            operand = blob[ipos:str_end].decode("latin-1", errors="replace")
            ipos = str_end + 1
            instructions.append((opcode, f'"{operand}"'))
        elif is_large(opcode):
            operand = struct.unpack(">i", blob[ipos:ipos+4])[0]
            ipos += 4
            instructions.append((opcode, operand))
        else:
            operand = blob[ipos]
            ipos += 1
            instructions.append((opcode, operand))

    result["instructions"] = instructions
    return result


def print_script(s, label=""):
    if label:
        print(f"\n{'='*60}")
        print(f"  {label}")
        print(f"{'='*60}")
    print(f"Name: {s['name']}")
    print(f"Source: {s['source_path']}")
    print(f"Lookup key: {s['lookup_key']}")
    print(f"Param types: {s['param_types']}")
    print(f"Line table ({len(s['line_table'])} entries):")
    for pc, line in s["line_table"]:
        print(f"  pc={pc} line={line}")
    print(f"Instructions ({s['instr_count']}):")
    for i, (op, operand) in enumerate(s["instructions"]):
        name = OPCODE_NAMES.get(op, f"CMD_{op}")
        print(f"  {i:4d}: {name}({op}) = {operand}")
    print(f"Locals: int={s['int_locals']} str={s['str_locals']}")
    print(f"Args: int={s['int_args']} str={s['str_args']}")
    if s["switches"]:
        print(f"Switches ({len(s['switches'])}):")
        for i, cases in enumerate(s["switches"]):
            print(f"  Table {i} ({len(cases)} cases):")
            for key, off in cases:
                print(f"    {key} -> {off}")


def compare_scripts(java, rust):
    """Compare two decoded scripts and print differences."""
    diffs = []

    if java["name"] != rust["name"]:
        diffs.append(f"Name: java={java['name']} rust={rust['name']}")

    if java["lookup_key"] != rust["lookup_key"]:
        diffs.append(f"Lookup key: java={java['lookup_key']} rust={rust['lookup_key']}")

    if java["param_types"] != rust["param_types"]:
        diffs.append(f"Param types: java={java['param_types']} rust={rust['param_types']}")

    # Line tables
    if java["line_table"] != rust["line_table"]:
        jlt = java["line_table"]
        rlt = rust["line_table"]
        diffs.append(f"Line table: java={len(jlt)} entries, rust={len(rlt)} entries")
        # Show first few differences
        max_show = max(len(jlt), len(rlt))
        for i in range(min(max_show, 60)):
            j = jlt[i] if i < len(jlt) else None
            r = rlt[i] if i < len(rlt) else None
            if j != r:
                diffs.append(f"  line[{i}]: java={j} rust={r}")

    # Instructions
    ji = java["instructions"]
    ri = rust["instructions"]
    if len(ji) != len(ri):
        diffs.append(f"Instruction count: java={len(ji)} rust={len(ri)}")
    for i in range(min(len(ji), len(ri))):
        if ji[i] != ri[i]:
            j_op, j_val = ji[i]
            r_op, r_val = ri[i]
            j_name = OPCODE_NAMES.get(j_op, f"CMD_{j_op}")
            r_name = OPCODE_NAMES.get(r_op, f"CMD_{r_op}")
            diffs.append(f"  instr[{i}]: java={j_name}({j_op})={j_val} rust={r_name}({r_op})={r_val}")

    # Trailer
    for field in ["int_locals", "str_locals", "int_args", "str_args"]:
        if java[field] != rust[field]:
            diffs.append(f"{field}: java={java[field]} rust={rust[field]}")

    if java["switches"] != rust["switches"]:
        diffs.append(f"Switches differ")

    return diffs


def main():
    args = sys.argv[1:]

    compare_mode = "--compare" in args
    if compare_mode:
        idx_pos = args.index("--compare")
        base_args = args[:idx_pos]
        compare_args = args[idx_pos+1:]
    else:
        base_args = args
        compare_args = []

    if len(base_args) < 3:
        print("Usage: decode_script.py <dat> <idx> <script_id> [--compare <dat2> <idx2>]")
        sys.exit(1)

    dat_path, idx_path = base_args[0], base_args[1]
    script_id = int(base_args[2])

    dat = read_dat(dat_path)
    entries = read_idx(idx_path)
    off, length = entries[script_id]
    script1 = decode_script(dat, off, length)

    if compare_mode and len(compare_args) >= 2:
        dat2 = read_dat(compare_args[0])
        entries2 = read_idx(compare_args[1])
        off2, length2 = entries2[script_id]
        script2 = decode_script(dat2, off2, length2)

        diffs = compare_scripts(script1, script2)
        if not diffs:
            print(f"Script [{script_id}] {script1['name']}: IDENTICAL")
        else:
            print(f"Script [{script_id}] {script1['name']}: {len(diffs)} differences:")
            for d in diffs:
                print(f"  {d}")
    else:
        print_script(script1, f"Script [{script_id}]")


if __name__ == "__main__":
    main()
