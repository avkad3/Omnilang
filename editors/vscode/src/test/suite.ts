// End-to-end checks, run inside VS Code by runTest.ts.

import * as assert from "assert";
import * as path from "path";
import * as vscode from "vscode";

const fixtures = path.resolve(__dirname, "../../test-fixtures");

async function open(name: string): Promise<vscode.TextEditor> {
  const doc = await vscode.workspace.openTextDocument(path.join(fixtures, name));
  return vscode.window.showTextDocument(doc);
}

/** Polls `check` until it returns a value or the timeout passes. */
async function eventually<T>(what: string, check: () => T | undefined | Promise<T | undefined>): Promise<T> {
  const deadline = Date.now() + 30_000;
  for (;;) {
    const value = await check();
    if (value !== undefined) {
      return value;
    }
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${what}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

async function configure(): Promise<void> {
  const config = vscode.workspace.getConfiguration("omni");
  await config.update("messageLanguage", "hi", vscode.ConfigurationTarget.Workspace);
  await config.update("typingLanguage", "hi", vscode.ConfigurationTarget.Workspace);
  // Changing the path restarts the server with these settings.
  await config.update("server.path", process.env.OMNI_SERVER, vscode.ConfigurationTarget.Workspace);
}

async function diagnosticsInHindi(): Promise<void> {
  const editor = await open("errors.om");
  const diags = await eventually("diagnostics", () => {
    const d = vscode.languages.getDiagnostics(editor.document.uri);
    return d.length > 0 ? d : undefined;
  });
  assert.strictEqual(diags[0].source, "omni");
  assert.ok(diags[0].message.includes("परिभाषित नहीं"), diags[0].message);
  // Line 2, after 4 spaces: positions survive the UTF-8/UTF-16 conversion.
  assert.strictEqual(diags[0].range.start.line, 1);
  assert.strictEqual(diags[0].range.start.character, 4);
  assert.strictEqual(diags[0].range.end.character, 9);
}

async function englishCompletesToHindi(): Promise<void> {
  const editor = await open("complete.om");
  const end = new vscode.Position(1, 9);
  const labels = await eventually("completion", async () => {
    const list = await vscode.commands.executeCommand<vscode.CompletionList>(
      "vscode.executeCompletionItemProvider",
      editor.document.uri,
      end,
    );
    const ls = list.items.map((i) => (typeof i.label === "string" ? i.label : i.label.label));
    return ls.includes("लौटाओ") ? ls : undefined;
  });
  assert.ok(labels.includes("लौटाओ"), labels.join(", "));
}

async function viewInTamil(): Promise<void> {
  await open("hello.om");
  await vscode.commands.executeCommand("omnilang.viewTamil");
  const text = await eventually("translated editor", () => {
    const t = vscode.window.activeTextEditor?.document.getText() ?? "";
    return t.includes("அச்சிடு") ? t : undefined;
  });
  // Keywords are translated; the string's contents are data and stay as written.
  assert.strictEqual(text, 'செயல்பாடு முதன்மை() {\n    "नमस्ते" அச்சிடு;\n}\n');
}

async function grammarHighlightsAllLanguages(): Promise<void> {
  const ext = vscode.extensions.getExtension("avkad3.omnilang");
  assert.ok(ext, "extension is installed");
  const grammarPath = path.join(ext.extensionPath, "syntaxes", "omnilang.tmLanguage.json");
  const grammar = JSON.parse((await vscode.workspace.fs.readFile(vscode.Uri.file(grammarPath))).toString());
  const control = grammar.patterns.find((p: { name?: string }) => p.name === "keyword.control.omnilang");
  // The pattern is Oniguruma syntax, which JavaScript's Unicode regexes share here.
  const re = new RegExp(control.match, "u");
  for (const word of ["if", "अगर", "என்றால்", "लौटाओ"]) {
    assert.ok(re.test(word), `${word} is highlighted`);
  }
  // A keyword inside a longer word must not match (Indic marks count as letters).
  assert.ok(!re.test("अगरबत्ती"), "no match inside a word");
}

/** Removes the workspace settings the tests wrote. */
async function restore(): Promise<void> {
  const config = vscode.workspace.getConfiguration("omni");
  for (const key of ["messageLanguage", "typingLanguage", "server.path"]) {
    await config.update(key, undefined, vscode.ConfigurationTarget.Workspace);
  }
}

export async function run(): Promise<void> {
  await configure();
  const tests: [string, () => Promise<void>][] = [
    ["diagnostics in Hindi at the right position", diagnosticsInHindi],
    ["English keyword completes to Hindi", englishCompletesToHindi],
    ["View in Tamil", viewInTamil],
    ["grammar covers every language", grammarHighlightsAllLanguages],
  ];
  let failed = 0;
  for (const [name, test] of tests) {
    try {
      await test();
      console.log(`  ok   ${name}`);
    } catch (err) {
      failed++;
      console.log(`  FAIL ${name}: ${err instanceof Error ? err.message : err}`);
    }
  }
  await restore();
  if (failed > 0) {
    throw new Error(`${failed} of ${tests.length} extension tests failed`);
  }
}
