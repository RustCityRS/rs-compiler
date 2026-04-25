#!/usr/bin/env python3
"""Compare individual script bytecodes between Java and Rust compiler output.

Reads script.dat + script.idx from both compilers and compares each script's
binary blob. Reports per-script match/mismatch and overall parity.

Usage:
    python3 compare_scripts.py <java.dat> <java.idx> <rust.dat> <rust.idx> [--verbose] [--diff N]
"""

import struct
import sys
import os


def read_idx(path):
    """Read script.idx -> list of (offset, length) for each script in script.dat."""
    with open(path, "rb") as f:
        data = f.read()
    count = struct.unpack(">I", data[0:4])[0]
    entries = []
    offset = 8  # skip 4-byte count + 4-byte version in .dat (version only in .dat)
    for i in range(count):
        length = struct.unpack(">I", data[4 + i * 4 : 8 + i * 4])[0]
        entries.append((offset, length))
        offset += length
    return entries


def read_dat(path):
    """Read the entire script.dat file."""
    with open(path, "rb") as f:
        return f.read()


def read_script_name(data, offset):
    """Read null-terminated script name from a script blob."""
    end = data.index(b"\x00", offset)
    return data[offset:end].decode("latin-1", errors="replace")


def strip_source_path(blob):
    """Strip the source file path from a script blob for path-independent comparison.

    Blob format: name\0source_path\0...rest...
    Returns: name\0\0...rest... (empty source path)
    """
    # Find end of script name
    name_end = blob.index(b"\x00")
    # Find end of source path
    path_end = blob.index(b"\x00", name_end + 1)
    # Return blob with empty source path
    return blob[:name_end + 1] + b"\x00" + blob[path_end + 1:]


def hex_dump(data, start=0, length=None, cols=16):
    """Return a hex dump string of binary data."""
    if length is None:
        length = len(data) - start
    lines = []
    for i in range(0, length, cols):
        chunk = data[start + i : start + i + cols]
        hex_part = " ".join(f"{b:02x}" for b in chunk)
        ascii_part = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        lines.append(f"  {i:06x}: {hex_part:<{cols*3}}  {ascii_part}")
    return "\n".join(lines)


def find_first_diff(a, b):
    """Find the byte offset of the first difference between two byte sequences."""
    min_len = min(len(a), len(b))
    for i in range(min_len):
        if a[i] != b[i]:
            return i
    if len(a) != len(b):
        return min_len
    return -1


def main():
    args = sys.argv[1:]
    verbose = "--verbose" in args
    if verbose:
        args.remove("--verbose")

    strip_paths = "--strip-paths" in args
    if strip_paths:
        args.remove("--strip-paths")

    diff_id = None
    if "--diff" in args:
        idx = args.index("--diff")
        diff_id = int(args[idx + 1])
        args.pop(idx)
        args.pop(idx)

    if len(args) < 4:
        print("Usage: compare_scripts.py <ref.dat> <ref.idx> <rust.dat> <rust.idx> [--verbose] [--strip-paths] [--diff N]")
        sys.exit(1)

    java_dat_path, java_idx_path, rust_dat_path, rust_idx_path = args[:4]

    java_dat = read_dat(java_dat_path)
    java_entries = read_idx(java_idx_path)
    rust_dat = read_dat(rust_dat_path)
    rust_entries = read_idx(rust_idx_path)

    java_count = len(java_entries)
    rust_count = len(rust_entries)

    print(f"Java scripts: {java_count}")
    print(f"Rust scripts: {rust_count}")

    if java_count != rust_count:
        print(f"WARNING: Script count mismatch! Java={java_count}, Rust={rust_count}")

    compare_count = min(java_count, rust_count)
    matches = 0
    mismatches = []

    for i in range(compare_count):
        j_off, j_len = java_entries[i]
        r_off, r_len = rust_entries[i]

        j_blob = java_dat[j_off : j_off + j_len]
        r_blob = rust_dat[r_off : r_off + r_len]

        # Read script name from Java blob
        name = read_script_name(j_blob, 0) if j_len > 0 else f"script_{i}"

        # Optionally strip source paths for path-independent comparison
        j_cmp = strip_source_path(j_blob) if strip_paths else j_blob
        r_cmp = strip_source_path(r_blob) if strip_paths else r_blob

        if j_cmp == r_cmp:
            matches += 1
        else:
            diff_offset = find_first_diff(j_cmp, r_cmp)
            mismatches.append((i, name, len(j_cmp), len(r_cmp), diff_offset))
            if verbose:
                print(f"  MISMATCH [{i}] {name}: java={j_len}B rust={r_len}B first_diff@{diff_offset}")

    # Handle diff mode
    if diff_id is not None:
        if diff_id >= compare_count:
            print(f"Script ID {diff_id} out of range (max {compare_count-1})")
            sys.exit(1)

        j_off, j_len = java_entries[diff_id]
        r_off, r_len = rust_entries[diff_id]
        j_blob = java_dat[j_off : j_off + j_len]
        r_blob = rust_dat[r_off : r_off + r_len]
        name = read_script_name(j_blob, 0) if j_len > 0 else f"script_{diff_id}"

        print(f"\n=== Diff for script [{diff_id}] {name} ===")
        print(f"Java: {j_len} bytes, Rust: {r_len} bytes")
        diff_off = find_first_diff(j_blob, r_blob)
        if diff_off == -1:
            print("Scripts are IDENTICAL")
        else:
            print(f"First difference at byte offset {diff_off}")
            context = 64
            start = max(0, diff_off - 16)
            print(f"\nJava bytes (offset {start}):")
            print(hex_dump(j_blob, start, min(context, j_len - start)))
            print(f"\nRust bytes (offset {start}):")
            print(hex_dump(r_blob, start, min(context, r_len - start)))
        return

    # Summary
    total = compare_count
    pct = (matches / total * 100) if total > 0 else 0
    print(f"\n{'='*60}")
    print(f"Bytecode Parity: {matches}/{total} ({pct:.2f}%)")
    print(f"Matches: {matches}, Mismatches: {len(mismatches)}")
    print(f"{'='*60}")

    if mismatches and not verbose:
        print(f"\nFirst 20 mismatched scripts:")
        for i, name, j_len, r_len, diff_off in mismatches[:20]:
            size_note = f" (java={j_len}B rust={r_len}B)" if j_len != r_len else f" ({j_len}B)"
            print(f"  [{i}] {name}{size_note} first_diff@{diff_off}")
        if len(mismatches) > 20:
            print(f"  ... and {len(mismatches) - 20} more")

    print(f"\nTo inspect a specific mismatch, run:")
    print(f"  python3 scripts/compare_scripts.py {java_dat_path} {java_idx_path} {rust_dat_path} {rust_idx_path} --diff <SCRIPT_ID>")

    # Exit with non-zero if not 100%
    sys.exit(0 if matches == total else 1)


if __name__ == "__main__":
    main()
