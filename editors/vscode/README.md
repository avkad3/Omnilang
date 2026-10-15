# OmniLang for VS Code

Write [OmniLang](https://github.com/avkad3/Omnilang) in English, Hindi or Tamil.

- **Romanized typing.** Type on a normal keyboard and pick the native word:
  - `agar` → `अगर`, `ginatii` → `गिनती`, `ninaivagam` → `நினைவகம்`.
  - Keywords also complete from their English name: in a Hindi file, `ret` offers `लौटाओ`.
- **Errors in your language.** Set `omni.messageLanguage` to `hi` or `ta`.
- **Hover** on any keyword to see it in every language, with its word order.
- **View in English / Hindi / Tamil** (Command Palette) opens a translated copy of the file beside it.
- **Format Document** tidies the file in its own language. Files that deliberately mix languages are left alone.
- **Highlighting** for every language pack, including Hindi's `।`.

## Setup

The extension runs the `omni` language server. Install it from the repository:

```bash
cargo install --path crates/omni-cli
```

If `omni` isn't on your `PATH`, set `omni.server.path` to the executable.

## Settings

| Setting | Default | |
|---|---|---|
| `omni.messageLanguage` | `en` | language of error messages |
| `omni.typingLanguage` | `en` | language for completion in a new, empty file |
| `omni.server.path` | `omni` | the `omni` executable |
