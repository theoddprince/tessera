The first public build of **Tessera**, a fast, native API client and a lightweight alternative to Postman, written in Rust.

## Download

- **`tessera-…-windows-x64.exe`**: the app itself. It's a single file with nothing to install, so download it and run it.
- **`tessera-…-windows-x64.zip`**: the same executable with the README and licences.
- **`SHA256SUMS.txt`**: checksums, so you can verify the download.

Windows 10 or 11 (x64) is required.

> **Windows SmartScreen:** the executable isn't code-signed yet, so Windows may show *"Windows protected your PC"* the first time you run it. Click **More info → Run anyway**.

## What's in this release

- **Requests**: any HTTP method; params, headers and bodies (raw, form, multipart, binary); highlighted JSON, a JSON tree, and search in the response.
- **Collections, environments and `{{variables}}`**, using Postman's precedence.
- **Auth**: Bearer, Basic, API key and OAuth 2.0 (Authorization Code with PKCE, Client Credentials, Password). Proxy, custom CA and client certificates are also supported.
- **Scripts and tests**: a Postman-compatible `pm` API (`pm.test`, `pm.expect`, `pm.environment`, …).
- **Flows**: wire requests together on a canvas and pass values such as a login token from one response to the next request.
- **Collection runner**: iterations and CSV/JSON data files.
- **GraphQL** with schema introspection, and **WebSocket** sessions.
- **Import / export**: imports Postman v2.0/v2.1 collections and environments and cURL commands, exports to Postman, and generates code snippets.
- **Interface**: command palette (Ctrl+K), dark and light themes, and tabs restored on restart.

Your data stays on your machine as plain JSON in `%APPDATA%\Tessera\Tessera\data`.

This is an **early alpha**: please report anything that breaks in [Issues](https://github.com/theoddprince/tessera/issues).
