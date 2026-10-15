// VS Code client for the OmniLang language server (`omni lsp`).

import * as vscode from "vscode";
import {
  ExecuteCommandRequest,
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

function settings() {
  const config = vscode.workspace.getConfiguration("omni");
  return {
    serverPath: config.get<string>("server.path", "omni"),
    messageLanguage: config.get<string>("messageLanguage", "en"),
    typingLanguage: config.get<string>("typingLanguage", "en"),
  };
}

async function start(context: vscode.ExtensionContext): Promise<void> {
  const s = settings();
  const serverOptions: ServerOptions = { command: s.serverPath, args: ["lsp"] };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: "omnilang" }],
    initializationOptions: {
      messageLanguage: s.messageLanguage,
      typingLanguage: s.typingLanguage,
    },
    // Sends workspace/didChangeConfiguration with { omni: {...} } on change.
    synchronize: { configurationSection: "omni" },
  };
  client = new LanguageClient("omnilang", "OmniLang", serverOptions, clientOptions);
  try {
    await client.start();
  } catch (err) {
    const choice = await vscode.window.showErrorMessage(
      `OmniLang: could not start "${s.serverPath} lsp". Install omni (cargo install --path crates/omni-cli) or set omni.server.path.`,
      "Open Settings",
    );
    if (choice === "Open Settings") {
      vscode.commands.executeCommand("workbench.action.openSettings", "omni.server.path");
    }
    client = undefined;
  }
}

/** Opens the active OmniLang file translated into `lang` in a new editor. */
async function viewIn(lang: string): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!client || !editor || editor.document.languageId !== "omnilang") {
    return;
  }
  const text: string | null = await client.sendRequest(ExecuteCommandRequest.type, {
    command: "omni.view",
    arguments: [editor.document.uri.toString(), lang],
  });
  if (text === null) {
    vscode.window.showWarningMessage("OmniLang: fix the errors in this file before translating it.");
    return;
  }
  const doc = await vscode.workspace.openTextDocument({ language: "omnilang", content: text });
  await vscode.window.showTextDocument(doc, vscode.ViewColumn.Beside);
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(
    vscode.commands.registerCommand("omnilang.viewEnglish", () => viewIn("en")),
    vscode.commands.registerCommand("omnilang.viewHindi", () => viewIn("hi")),
    vscode.commands.registerCommand("omnilang.viewTamil", () => viewIn("ta")),
    vscode.commands.registerCommand("omnilang.restartServer", async () => {
      await client?.stop();
      await start(context);
    }),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      // The server path can only change by restarting the server.
      if (e.affectsConfiguration("omni.server.path")) {
        await client?.stop();
        await start(context);
      }
    }),
  );
  await start(context);
}

export async function deactivate(): Promise<void> {
  await client?.stop();
}
