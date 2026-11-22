# Getting started with viscurl

Open an OpenAPI file, choose an endpoint, and send a request from your terminal. The local example below gives you a working API to try if you are starting from scratch.

## Run or install

Use a current stable Rust toolchain with Cargo and an interactive terminal. A terminal with at least 30 columns and 8 rows can display the app. Wider windows show more panes at once.

Run viscurl from the repository root with the path to your spec.

```sh
cargo run --locked -- /path/to/openapi.yaml
```

To install the command, run the following from the repository root. Once Cargo's binary directory is on your PATH, you can run `viscurl` from any directory.

```sh
cargo install --locked --path .
viscurl /path/to/openapi.json
```

## Send your first request

If you already have an API running, open its spec and start at step 3. To try the bundled example, use two terminals and run both commands from the repository root.

1. In the first terminal, start a local server with Python 3.

   ```sh
   python3 -m http.server --bind 127.0.0.1 --directory examples 8765
   ```

2. In the second terminal, open the sample spec.

   ```sh
   cargo run --locked -- examples/quickstart.yaml
   ```

3. In **Endpoints**, press **Enter** on a folder to expand it. For the sample, open **Quickstart**, move down to **GET /hello.json**, and press **Enter** to open **Request**. To find an operation in a larger spec, press **/** and enter part of its path, method, tag, or summary.
4. Check the destination at the top of the screen. If you are using your own spec and need a different server, edit the **Server** field and enter the absolute HTTP(S) URL. Press **Ctrl+S** when you are ready to send.
5. The **Response** pane takes focus. The sample returns status **200**, marked **Documented**, and the JSON body below. Press **Enter** to switch to **Headers**, then again for **Raw**.

   ```json
   {
     "message": "Hello from viscurl"
   }
   ```

If port 8765 is in use, start the sample server on an unused port and set **Server** in viscurl to that port. Press **q** to quit viscurl and **Ctrl+C** in the first terminal to stop the sample server.

## Move between panes

**Tab** moves from **Endpoints** to **Request** to **Response**. Use **Shift+Tab** to go back. On a narrow terminal, this also switches the visible pane. Each endpoint keeps its own draft during the session, so you can move around without losing edits.

See [Requests and variables](requests-and-variables.md) for request bodies, headers, and credentials, or [Keyboard and troubleshooting](keyboard-and-troubleshooting.md) for the full key reference.
