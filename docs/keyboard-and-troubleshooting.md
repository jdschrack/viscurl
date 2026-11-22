# Keyboard and troubleshooting

## Keyboard reference

| Where | Key | Action |
| --- | --- | --- |
| Workspace | **Tab** / **Shift+Tab** | Move focus between panes |
| Workspace | **/** | Search endpoint paths, methods, summaries, and tags |
| Workspace | **Esc** | Close the current dialog or clear endpoint search |
| Workspace | **v** / **!** / **?** | Open variables, import warnings, or keyboard help |
| Workspace | **q** / **Ctrl+C** | Quit. Use **Ctrl+C** from an open editor |
| Endpoints | **↑** / **↓** | Move between folders and operations |
| Endpoints | **Enter** | Toggle a folder or open an operation |
| Endpoints | **←** / **→** | Collapse or expand a folder. **←** on an operation moves to its parent |
| Request | **↑** / **↓** | Move between fields |
| Request | **Enter** | Open a field editor or cycle the content type or auth scheme |
| Request | **←** / **→** | Cycle server, content type, or auth choices |
| Request | **Ctrl+S** | Save the open multiline editor or send the selected request |
| Response | **Enter** | Cycle **Body → Headers → Raw** |
| Response | **↑** / **↓** / **←** / **→** | Scroll vertically or horizontally |
| Response | **[** / **]** | View an older or newer response for the same endpoint |
| Variables | **a** / **Enter** / **r** / **Delete** | Add a pair, edit a value, rename a key, or remove a pair |
| Variables table | **Ctrl+S** | Save variables for future sessions |

Use **Page Up** and **Page Down** to move faster in endpoint, request, response, and variable lists. In a single-line request editor, **Enter** applies the value.

## Read a response

The response summary shows its status, elapsed time, and preview size. **Documented** means the status matches a declared code, status range, or default response in the spec. **Body** shows formatted JSON when available. **Raw** shows the unformatted text preview. **Headers** shows the returned headers, with authorization and cookie values masked.

viscurl keeps the last eight results, including failed requests, across all endpoints during the session. A request has a 15-second timeout. A redirect appears as its own HTTP response. Use **Headers** to inspect its `Location` value. Body previews show up to 256 KiB.

## Common problems

| What you see | What to check |
| --- | --- |
| **Enter an absolute HTTP(S) server URL** | Edit **Server**. Use a URL such as `http://localhost:8080` and put any required query values in **Extra query** or the documented query fields. |
| **Set variable … in Variables (v)** | Press **v** and check the key spelling. Use `{{name}}` in request values, or `{name}` when the single token fills the whole **Server** field. |
| **Could not connect to the server: …** | The text after the colon gives the cause, such as a refused connection or an untrusted certificate. Confirm the API is running and that the scheme, hostname, and port match the destination shown in the top bar. |
| **401** or **403** | Check the selected **Auth** scheme and the credential's access to that operation. |
| **Blank body or header input** | Check the selected content type or header parameter for a documented example or default. Open **Body** for an available JSON schema starter, or enter the expected value yourself. |
| **Response headers are missing from view** | Focus **Response** with **Tab** and press **Enter** until the pane title says **Headers**. |
| **An endpoint is missing** | Clear the filter with **Esc**, expand its folder, and press **!** to check for import warnings. |
| **Saved variables fail to load** | Check the [config file location](requests-and-variables.md#reuse-values-across-requests) and its permissions. The file uses a JSON object with string values. Correct the file and restart viscurl. |

## Supported scope

viscurl reads local OpenAPI 3.0, 3.1, and 3.2 YAML or JSON files up to 4 MiB. Features added in 3.2, such as the `query` method and `querystring` parameters, are skipped. It resolves supported same-file references. Bundle specs with external references before opening them. The request editor supports scalar parameters, form-style query arrays, and JSON or raw text bodies. Response checks cover HTTP status codes. Full schema validation, scripted assertions, file uploads, and saved collections are outside the beta scope.
