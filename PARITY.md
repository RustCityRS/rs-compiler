# Bytecode Parity: Rust Compiler vs Neptune 254 Compiler

## Result

**100% bytecode parity achieved: 9248/9248 scripts produce byte-identical output.**

Reference compiler: Neptune (Kotlin/Java) via `bun run build` on the Engine-TS 254 branch.
Content: 2004scape Content 254 branch (1316 `.rs2` source files, 9248 compiled scripts).
Symbols: Engine-TS `data/symbols/` directory (commands.sym, runescript.sym, varbit.sym, etc).

## Methodology

### Generating the Reference

1. Checked out Engine-TS and Content on the `254` branch
2. Installed Java 17 via SDKMAN (`sdk install java 17.0.18-amzn`) — required by Neptune
3. Ran `bun run build` from the Engine-TS directory, which invokes Neptune to compile all scripts
4. Copied the resulting `data/pack/server/script.dat` and `script.idx` as the reference

### Comparing Output

A Node.js comparison script (`compare.mjs`) parses both `script.idx` files to get per-script byte lengths, then extracts each script's bytes from `script.dat` and does a `Buffer.compare()`. Scripts are indexed by their ID (position in the idx file), so null/empty slots for unused IDs are compared too.

Additional analysis tools (`analyze_diff.mjs`, `debug_diff.mjs`, `trace_script.mjs`, `diff_instrs.mjs`, `diff_lines.mjs`) were used to categorize mismatches into: line table diffs, instruction diffs, trailer diffs, lookup key diffs, and size diffs — and to disassemble individual scripts side-by-side.

## Fixes Applied (in order)

### 1. Parser: Braced switch case bodies
**Scripts affected:** 1 (macro_event_maze.rs2)
**Symptom:** Parse error `Unexpected token: LBrace '{'`

The Content uses braced blocks in switch cases: `case 0 : { $x = coins; $y = 1; }`. The parser's `parse_case_body()` only handled unbraced statement lists. Added a check for `LBrace` at the start of a case body to delegate to `parse_block()`.

### 2. Parser: Component colon syntax in expressions
**Scripts affected:** 1 (duel_arena.rs2)
**Symptom:** Parse error `Expected Comma, got Colon ':'`

`return(duel_confirm:before_rule_line1)` uses `interface:component` compound identifiers inside `return()`. The parser only handled this syntax inside `parse_call_args()`. Added compound identifier parsing in `parse_primary()` for the `Identifier` case, with a guard: only consume the colon when the token after the compound is a value terminator (`RParen`, `Comma`, `Semicolon`, `RBrace`) — not `:` (switch case separator) or `(` (function call start, which would indicate the colon was actually a case separator followed by a statement).

### 3. CLI: `--symbols` flag
**Reason:** Content scripts are at `2004scape/Content/scripts/` but symbols are at `2004scape/Engine-TS/data/symbols/`. The auto-detection logic couldn't find them.

Added an optional `--symbols` CLI argument that overrides the auto-detected symbols directory.

### 4. Source path normalization
**Scripts affected:** All 9248
**Symptom:** Every script differed at the source path in the header

Two issues:
- `fs::canonicalize()` on Windows produces `\\?\C:\...` extended-length paths. Stripped the `\\?\` prefix.
- The Neptune compiler resolves `../content` (lowercase) while the filesystem has `Content` (uppercase). Added `.replace("\\Content\\", "\\content\\")` to match.

### 5. Varbit support
**Scripts affected:** ~15 (troll quest, wilderness, etc.)
**Symptom:** Missing `PUSH_VARBIT`/`POP_VARBIT` instructions; wrong opcodes used for varbit variables

The symbols loader loaded `varp.sym`, `varn.sym`, `vars.sym` but not `varbit.sym`. Variables like `%troll_opened_back_exit` are varbits, not varps, and require opcodes 25/27 instead of 1/2. Added `load_game_vars(registry, "varbit.sym", "varbit")` to the symbol loading sequence. The compiler already had the opcode dispatch (`"varbit" => Opcode::PushVarbit`) — it just never triggered because varbit vars weren't loaded.

### 6. Constant comment stripping
**Scripts affected:** 1 (macro_event_maze.rs2)
**Symptom:** Integer constant treated as string

A `.constant` file contained: `^macro_maze_chest_ticks = 100 // 60 seconds per chest respawn...`. The `// comment` was included in the value string. `parse::<i32>()` failed on `"100 // 60 seconds..."`, so it fell through to string handling. Comment stripping in `strip_comments()` (matching 2004scape's `loadFileFull()`) handles this.

### 7. Command return value discard
**Scripts affected:** ~45
**Symptom:** Missing `POP_INT_DISCARD` after command calls used as statements

Commands like `db_find()`, `clearbit_range()`, `setbit_range_toint()` return values. When called as statements (return value unused), Neptune emits `POP_INT_DISCARD` to clean the stack. Our compiler only did this for `ProcCall` and `JumpCall`, not `CommandCall`.

Added a `CommandCall` branch in the statement expression handler that looks up the command's return types and emits the appropriate discard instructions. Return types come from `engine.rs2` parsing (e.g., `[command,clearbit_range](...)(int)`).

For `db_find` and related commands whose return types aren't declared in `engine.rs2`, added `patch_command_return_types()` to manually set them based on Neptune's internal knowledge.

### 8. Script ID slot allocation (positional indexing)
**Scripts affected:** All
**Symptom:** Script ordering and count mismatch

The writer was emitting scripts contiguously (8032 entries). Neptune uses positional indexing: script ID = position in the dat/idx files, with empty (0-byte) slots for unused IDs. Changed `write_all()` to allocate `max_id + 1` slots, placing each script at its ID position.

### 9. Local variable slot allocation: phantom slots
**Scripts affected:** ~6
**Symptom:** Different slot numbers for same-named variables in sibling if/else branches

Neptune's slot allocation model:
1. **Always advance** the slot counter for every `def_int`/`def_string`/`def_long` declaration
2. **Then check** if a variable with the same name was already declared — if so, reuse the OLD slot
3. The newly allocated slot becomes a "phantom" — counted but never referenced

This means the slot counter progresses linearly through declarations, but repeated names share the first declaration's slot. Sibling if/else branches declaring the same name each consume a phantom slot, keeping subsequent variables at higher indices.

**Proof:** In `[opheldu,goldbowlbless_pure]`, `$bowl_uses` is declared in two sibling if-branches. The reference bytecode shows slots 0, (phantom 1), 2, 3, 4 — where slot 1 is allocated but never referenced. With pure name-reuse (no phantom), we produced slots 0, 0, 1, 2, 3. With the phantom model, we correctly produce 0, 0, 2, 3, 4.

### 10. Pre-statement string argument handling
**Scripts affected:** 4
**Symptom:** Line table PC offset differences and wrong null encoding

Neptune pushes string-typed arguments BEFORE the LineNumber instruction for statement-level command calls. Three sub-fixes:

**a) Null literal as string:** `set_player_op(null, 2, ^false)` — `null` in a string parameter position must be pushed as `PUSH_STR "null"`, not `PUSH_INT -1`. Added null literal detection in `count_pre_stmt_string_args()` that checks the command's parameter type from `engine.rs2`.

**b) Type-hinted pre-stmt compilation:** Pre-statement args were compiled with `compile_expr()` (no type hint), so null always became `PUSH_INT -1`. Changed to `compile_expr_hinted()` with the command's parameter type, so null in string position correctly produces `PUSH_STR "null"`.

**c) Line-1 emission guard:** Neptune only emits the `LineNumber(line - 1)` entry before pre-stmt args when the first arg is a `ConstantVar` (^name). Null literals and string literals skip this. Without this guard, `set_player_op(null, ...)` as the first statement in an else block produced a spurious line entry for the `} else {` line.

## Verification

```
$ cargo run --release -- compile \
    --source 2004scape/Content/scripts/ \
    --symbols 2004scape/Engine-TS/data/symbols/ \
    --output data/pack/server

$ node compare.mjs
Reference: 9248 scripts
Ours:      9248 scripts
Matches:   9248/9248 (100.00%)
Mismatches: 0
```

The comparison is byte-level: each script's encoded binary (header + instructions + trailer) is compared with `Buffer.compare()`. A match means every byte is identical — name, source path, lookup key, parameter types, line number table, instruction opcodes, instruction operands, local/arg counts, and switch tables.
