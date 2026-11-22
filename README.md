# viscurl

viscurl is a keyboard-first terminal client for exploring an OpenAPI spec and sending live HTTP requests.

```sh
cargo run --locked -- ./openapi.yaml
cargo run --locked -- ./openapi.json
```

Use an interactive terminal. On a wide screen, the endpoint list, request editor, and response inspector appear together. On a narrow screen, Tab switches between them.

| Key | Action |
| --- | --- |
| `/` | Search paths, methods, summaries, and tags |
| `↑` / `↓` | Move between endpoints or fields, or scroll a response |
| `Tab` / `Shift+Tab` | Change focus |
| `Enter` | Open an endpoint, edit a field, or switch response views |
| `←` / `→` | Cycle server, content type, or auth choices, or scroll the response horizontally |
| `Ctrl+S` | Save an open editor or send the current request |
| `[` / `]` | View an older or newer response for the selected endpoint |
| `!` | Show spec import warnings |
| `?` | Show keyboard help |
| `q` / `Ctrl+C` | Quit |

### Try a request

1. Open an endpoint with Enter. The request starts with its documented server, parameters, and first body example, when available. Edit **Server** to use another absolute HTTP(S) URL.
2. Edit the inputs and body. In **Extra query** and **Headers**, use one `name: value` per line. Use **Auth** to choose Basic, Bearer, or an API-key scheme from the spec, then fill in the masked credential field.
3. Confirm the destination in the top bar and send with Ctrl+S. Use Enter in the response pane to switch between the formatted body, headers, and raw text. The response indicates whether its HTTP status is documented.

### Beta scope

- Local OpenAPI 3.0 and 3.1 YAML or JSON files, including same-file `$ref` references used to build requests. Bundle specs that reference other files before importing.
- Scalar parameters, form-style query arrays entered as JSON, and JSON or raw text request bodies. For fields marked `†`, enter a serialized value. Use **Extra query** for repeated keys or query objects that expand to separate keys. Supply a pre-encoded segment for complex path parameters. Query values use ordinary form URL encoding.
- Basic and Bearer auth, plus API keys declared in the spec. Use the available header, cookie, and query fields for combined schemes. Enter OAuth access tokens as Bearer credentials.
- Drafts, credentials, and the eight most recent responses stay in memory. Requests time out after 15 seconds. Redirects are displayed as responses. Response previews are limited to 256 KiB, and local spec files to 4 MiB.
- Response checks cover documented HTTP status codes. Schema validation, scripted assertions, file uploads, and saved collections are outside the beta scope.

Run the checks with `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings`. Equivalent YAML and JSON samples are in `tests/fixtures`.
