# reelforge-intelligence (CLI)

Host binary for **ReelForge Intelligence** — no FFmpeg, no model weights.

```bash
cargo run -p reelforge-intelligence-cli -- --help
```

## Commands

| Command | Role |
| --- | --- |
| `methods` | List MCP method names |
| `dispatch --method M --args '{...}'` | One-shot JSON call |
| `serve` | Line-delimited stdio MCP host |
| `resolve-bridge` | Load SightLoom package + intent → freeze → live graph JSON (`--bindings` rewrites FramePick) |
| `version` | Print crate version |

## Stdio protocol (`serve`)

One JSON object per line on stdin:

```json
{"id":1,"method":"operations","args":{}}
```

Stdout:

```json
{"id":1,"ok":true,"result":[...]}
```

or

```json
{"id":1,"ok":false,"error":"..."}
```

Empty line / `{"method":"shutdown"}` exits.

## Example: package → bridge

```bash
cargo run -p reelforge-intelligence-cli -- resolve-bridge \
  --package /path/to/vision_index_package \
  --plan intent.json \
  --output out.mp4 \
  --write-graph graph.json \
  --style gaussian
```

`--style`: `gaussian` (default) | `pixelate` | `solid`. Privacy hosts should pick `pixelate`.

## MCP methods

See `reelforge_intelligence_core::list_methods` / `dispatch`.
