// Launches a real VS Code with the extension and runs suite.ts inside it.
// The language server is the `omni` binary from this repository's build;
// set OMNI_SERVER to use another one.

import * as path from "path";
import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const root = path.resolve(__dirname, "../../");
  const repo = path.resolve(root, "../../");
  const exe = process.platform === "win32" ? "omni.exe" : "omni";
  const server = process.env.OMNI_SERVER ?? path.join(repo, "target", "debug", exe);
  await runTests({
    extensionDevelopmentPath: root,
    extensionTestsPath: path.join(__dirname, "suite.js"),
    launchArgs: [path.join(root, "test-fixtures"), "--disable-extensions", "--disable-workspace-trust"],
    extensionTestsEnv: { OMNI_SERVER: server },
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
