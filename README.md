<div align="center">
<pre>
██████╗ ██╗   ██╗███████╗████████╗ ██████╗██╗████████╗██╗   ██╗
██╔══██╗██║   ██║██╔════╝╚══██╔══╝██╔════╝██║╚══██╔══╝╚██╗ ██╔╝
██████╔╝██║   ██║███████╗   ██║   ██║     ██║   ██║    ╚████╔╝ 
██╔══██╗██║   ██║╚════██║   ██║   ██║     ██║   ██║     ╚██╔╝  
██║  ██║╚██████╔╝███████║   ██║   ╚██████╗██║   ██║      ██║   
╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝    ╚═════╝╚═╝   ╚═╝      ╚═╝   
</pre>
</div>

----

# RuneScape Compiler (runec)

`runec` compiles RuneScript (`.rs2`) for the server and ClientScript (`.cs2`)
for the client. GlobalScript (`.gs2`) holds code shared by both. It also provides
an editor language server and a Rust library with ClientScript decompilation.

The examples assume `runec` is installed and on your `PATH`. Run commands from
your project's root directory and adjust the source, pack and output paths to
match your project.

## Project layout

A project with server, client and shared code can use this layout:

```text
content/
├── scripts/
│   ├── runescript/       # .rs2 files and engine.rs2
│   ├── clientscript/     # .cs2 files and engine.cs2
│   └── globalscript/     # shared .gs2 files
└── pack/                 # script IDs, command opcodes and named assets
```

Both compilers automatically include the sibling `globalscript` directory.
Put configs and constants alongside the scripts that own them. Server and client
code can use their own definitions and shared definitions; shared code can only
use shared definitions. Flat source directories are also supported.

Your project supplies the engine commands, asset names and script IDs.
`engine.rs2` and `engine.cs2` declare command parameters and return types.
The pack directory maps names to IDs:

| Files | Purpose |
|---|---|
| `command.pack` | Server command opcodes; server script IDs are generated in memory |
| `cs2_command.pack` and `clientscript.pack` | Client opcodes and script IDs |
| Other packs, such as `obj.pack` and `inv.pack` | Named assets and game data |

Client opcode mappings come from the project, so there is no client revision
flag to set.

## Compile server scripts

```bash
runec compile --source content/scripts/runescript --pack content/pack --output data/server
```

This reads the server and shared scripts, checks them, and writes `script.dat`
and `script.idx`. Batch compilation updates the script ID registry.

Optional flags:

| Flag | Effect |
|---|---|
| `--lint` | Also report unused locals and unreachable code |
| `--low-mem` | Use less memory at the cost of more compilation work |

If you omit `--output`, server output goes to `./data/pack/server`.
`--source` and `--output` can also be written as `-s` and `-o`.

## Compile client scripts

```bash
runec compile-client --source content/scripts/clientscript --pack content/pack --output data/clientscript
```

This reads the client and shared scripts and writes one `<id>.dat` file per
script. Existing IDs are preserved; successful compilation adds new scripts to
`clientscript.pack`.

Keep explicit ID entries when renaming scripts. For example, renaming
`clientscript_1465` to `bank_refresh` requires `1465=[proc,bank_refresh]` in
`clientscript.pack` to preserve that client's script ID.

## Check server scripts

```bash
runec lint --source content/scripts/runescript --pack content/pack
```

This reports compiler errors, pointer warnings, unused locals and unreachable
code without writing bytecode output. Client diagnostics are available during
`compile-client` and through the ClientScript language server.

Use `--help` for all available options:

```bash
runec --help
runec compile --help
runec compile-client --help
runec lsp --help
```

## Hooks and client callbacks

Hooks take a quoted callback expression. Arguments inside it are code, so
variables, constants and nested calls work normally:

```rs2
if_setontimer("bank_v2_rebuild_delayed(%varc_193)", bank_v2_main:com93);
if_setontimer(null, bank_v2_main:com93);
```

The first call sets the hook; `null` clears it. Empty callback parentheses are
optional: `"callback"` and `"callback()"` both mean no arguments.

Transmit hooks add a list of triggers inside the quotes:

```rs2
if_setonvarctransmit("clientscript_1465(){varc_192, varc_1038, varc_1324}", bank_v2_main:com21);
```

Use bare names in `{...}`. These refer to trigger IDs, not the current values of
the variables. Depending on the hook, the names identify varps, integer or string
varcs, inventories, or stats.

Server scripts use the same callback syntax with `runclientscript`:

```rs2
runclientscript("playerdesign4_submit_name(0, 1, 1)");
```

The compiler reads the sibling ClientScript sources to resolve callbacks and
check their arguments. `runclientscript` takes one quoted callback; the old
`runclientscript*` and unquoted forms are errors.

String literals inside a callback need escaped quotes:

```rs2
runclientscript("show_message(\"Hello\")");
```

This example assumes a client callback named `show_message` that takes a string.
The editor understands callback names, arguments and named transmit triggers
for navigation, completion, highlighting and inlay hints.

## Editor setup

Use your editor's LSP client to launch `runec` over stdio. Configure separate
servers for RuneScript and ClientScript, with your project root as the working
directory.

RuneScript server:

```bash
runec lsp --source content/scripts/runescript --pack content/pack
```

ClientScript server:

```bash
runec lsp --client --source content/scripts/clientscript --pack content/pack
```

If the editor has separate fields for the executable and arguments, use `runec`
as the executable and everything after it as the arguments. Leave initialization
options blank for this directory-based setup. If the editor cannot find `runec`,
use the full path to the installed executable.

Map files to the servers as follows:

| Server | File pattern | Language ID |
|---|---|---|
| RuneScript | `*.rs2` | `rs2` |
| RuneScript | `*.gs2` | `gs2` |
| ClientScript | `*.cs2` | `cs2` |
| ClientScript | `*.gs2` | `gs2` |

The language server provides diagnostics, completion, hover, definitions,
references, rename, syntax highlighting, signature help, inlay hints, formatting,
quick fixes and Extract Proc. Changes refresh diagnostics in affected open files.

Config and pack files use language ID `rs2config` for navigation and editing. Any
file mapped to it opens as a config file, so add a pattern for each extension in
use: `*.anim`, `*.area`, `*.bas`, `*.billboard`, `*.category`, `*.varconbit`, `*.constant`,
`*.controller`, `*.cursor`, `*.dbrow`, `*.dbtable`, `*.defaults`, `*.enum`, `*.flo`, `*.flu`,
`*.fontmetrics`, `*.hashes`, `*.hitmark`, `*.hunt`, `*.idk`, `*.if`, `*.if3`, `*.inv`, `*.itemcode`,
`*.jingle`, `*.light`, `*.loc`, `*.locshape`, `*.mel`, `*.mesanim`, `*.midi`,
`*.model`, `*.msi`, `*.npc`, `*.obj`, `*.order`, `*.overlayinterface`, `*.pack`,
`*.param`, `*.particleeffector`, `*.particleemitter`, `*.quest`, `*.quickchatcat`,
`*.quickchatphrase`, `*.seq`, `*.spotanim`, `*.stat`, `*.struct`, `*.synth`,
`*.varpbit`, `*.varc`, `*.varclan`, `*.varclansetting`, `*.varcon`,
`*.varn`, `*.varnbit`, `*.varobj`, `*.varp`, `*.vars` and `*.wma`.

```
*.anim;*.bas;*.category;*.config15;*.config18;*.config19;*.config20;*.config21;*.config22;*.config23;*.config24;*.config25;*.config37;*.config38;*.config39;*.config40;*.config41;*.config42;*.config43;*.varconbit;*.config45;*.config47;*.config48;*.config7;*.constant;*.cursor;*.dbrow;*.dbtable;*.enum;*.flo;*.flu;*.fontmetrics;*.hashes;*.hitmark;*.hunt;*.idk;*.if;*.inv;*.jingle;*.light;*.loc;*.locshape;*.mel;*.mesanim;*.midi;*.model;*.msi;*.npc;*.obj;*.order;*.overlayinterface;*.pack;*.param;*.quest;*.seq;*.spotanim;*.stat;*.struct;*.synth;*.varpbit;*.varc;*.varclansetting;*.varn;*.varp;*.vars;*.billboard;*.controller;*.defaults;*.area;*.if3;*.itemcode;*.particleeffector;*.particleemitter;*.quickchatcat;*.quickchatphrase;*.varclan;*.varcon;*.varnbit;*.varobj;*.wma;*.fm;*.fms```

Optional color settings:

- **JetBrains with LSP4IJ:** import [editor/runec.icls](editor/runec.icls)
  under **Editor > Color Scheme**, then select `runec`.
- **VS Code:** merge [editor/semantic-colors.json](editor/semantic-colors.json)
  into your settings.

The language server does not write source files or ID packs. Restart the
configured servers after updating `runec` so they load the new executable.

## Rust integrations

The package is `rs-runec`; Rust imports use `runec`.

```toml
[dependencies]
rs-runec = "0.5"
```

Use [Project](src/project.rs) to manage source documents in memory:

| Method | Purpose |
|---|---|
| `open` / `update` / `close` | Manage source documents |
| `analyze` | Parse, register symbols and check types |
| `check` | Also check code generation, pointers and lints |
| `compile` | Return compiled artifacts and diagnostics |

The host supplies a [SymbolRegistry](src/symbol.rs) and owns storage and script
ID persistence. [EngineProfile](src/environment.rs) describes engine-specific
triggers and pointer rules.

For ClientScript tools, use [Format](src/clientscript/format.rs) for opcode
mappings, [Script::decode](src/clientscript/decode.rs) for binary decoding, and
the [decompiler](src/clientscript/decompile.rs) for source generation.
Decompilation is a library API, not a CLI command.

Custom LSP hosts can run `runec lsp` without `--source` and supply their
environment through `initializationOptions`. See
[project_from_json](src/lsp.rs) for the supported fields.

## License

[MIT](LICENSE).
