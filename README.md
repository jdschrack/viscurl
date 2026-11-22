# viscurl

viscurl is a keyboard-first terminal client for exploring OpenAPI specs and sending HTTP requests. Browse endpoints in collapsible folders, edit requests and credentials, and inspect status, timing, response headers, and JSON bodies.

## Open a spec

From a checkout of this repository, run the app in an interactive terminal.

```sh
cargo run --locked -- ./openapi.yaml
```

viscurl reads local OpenAPI 3.0, 3.1, and 3.2 files in YAML or JSON. Press **?** for keyboard help and **Ctrl+S** to send a request.

## User guides

- Install viscurl, send a request to a local sample server, and find the response headers. [Getting started](docs/getting-started.md)
- Build requests, use schema starters and authentication, and save reusable variables. [Requests and variables](docs/requests-and-variables.md)
- Look up shortcuts, troubleshoot errors, and check what the beta supports. [Keyboard and troubleshooting](docs/keyboard-and-troubleshooting.md)

## License

[MIT](LICENSE)
