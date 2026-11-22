# Requests and variables

Open an endpoint to edit its draft. The request fields are seeded from the operation's OpenAPI definition, and you can override them for the API environment you want to test.

## Server and request fields

- **Server** uses the first server available for the operation. Press **←** or **→** on the row to cycle through the defined servers, or press **Enter** to type an absolute HTTP(S) URL. Include the base path when your API needs one.
- Path, query, header, and cookie parameters appear as separate fields, such as **header · X-Request-ID**. Inline examples and schema defaults can supply their initial values. An asterisk marks a required field. For fields marked **†**, enter a serialized value. For complex path values, use a pre-encoded path segment.
- **Extra query** and **Headers** accept one pair per line in `name: value` format. Press **Ctrl+S** to save either editor. Repeat a name on separate lines to send repeated query values. In a documented form-style query array field, enter a JSON array such as `["pending", "active"]`.
- **Content type** cycles through the operation's body media types and a raw option. In raw mode, set `Content-Type` in **Headers** when the server requires it.

## Request bodies

Open **Body** with **Enter**. For the selected content type, viscurl loads the first available inline example, including optional fields present in that example. It can also use a top-level schema example or default.

For schema-only JSON requests, the editor opens with a **Schema starter** when viscurl can infer a shape from the schema. It follows local references and uses field types to create editable placeholders. For objects with a nonempty `required` list, the starter includes the listed fields. Otherwise it includes the defined writable fields. It skips `readOnly` fields and applies the same rules to nested objects.

Fill in values that make sense for your API, then press **Ctrl+S** to save the draft. The generated starter becomes the request body when saved. **Esc** closes the editor and discards edits made since opening it. Use **Enter** or paste directly to enter multiline JSON.

## Authentication

Select **Auth** and press **Enter** or use **←** or **→** to choose a scheme. Enter credentials in the masked field that appears below it.

| Scheme | Credential |
| --- | --- |
| Basic | Enter `username:password` |
| Bearer | Enter the token itself. viscurl adds the `Bearer` prefix |
| API key | Enter the key. viscurl uses the header, query, or cookie location declared by the spec |

For an API that requires multiple schemes together, supply the additional values through the relevant request fields. An OAuth access token you already have can be entered as a Bearer token. Credentials entered directly in **Auth** stay in memory for the session.

## Reuse values across requests

1. Press **v** to open the variables table.
2. Press **a** to add a row. Enter a key such as `base_url`, press **Enter**, enter a value such as `http://localhost:8080`, and press **Enter** again.
3. Press **Esc** to return to the workspace. Set **Server** to `{base_url}` or `{{base_url}}`. The top bar shows the resolved destination.

Use double braces when combining a token with other text, as in `{{base_url}}/api`. Double-brace tokens also work in parameter values, extra query and header values, the request body, and auth input. Substitution inserts text once. In a JSON body, write the surrounding quotes and give the variable text that produces valid JSON in that position. For example, enter `X-Workspace: {{workspace}}` in **Headers** to reuse a workspace value. An undefined token stops the request and identifies the missing variable.

viscurl fills variables only into text you saved from an editor. Values prefilled from the spec, such as examples and defaults, are sent exactly as written, so a spec file cannot pull your saved variables into a request. To use a token in a prefilled value, open the field and save it.

In the variables table, use **Enter** to edit a value, **r** to rename a key, and **Delete** to remove a row. Edits apply to the current session. To reuse them after restarting, press **Ctrl+S** in the table.

Saved variables are shared across specs and stored as plaintext. The config file is `$XDG_CONFIG_HOME/viscurl/variables.json` when an absolute XDG config directory is set. The usual default is `~/.config/viscurl/variables.json` on Unix-like systems and `%APPDATA%/viscurl/variables.json` on Windows.
