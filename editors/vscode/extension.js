// Lunar for VS Code: starts `lunar lsp` and connects it to .ln files.

const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

/** @type {LanguageClient | undefined} */
let client;

function createClient() {
  const config = vscode.workspace.getConfiguration("lunar");
  const command = config.get("server.path") || "lunar";
  const run = { command, args: ["lsp"], transport: TransportKind.stdio };
  return new LanguageClient(
    "lunar",
    "Lunar Language Server",
    { run, debug: run },
    {
      documentSelector: [{ scheme: "file", language: "lunar" }],
      // Also resolves `lunar.trace.server`.
      outputChannelName: "Lunar Language Server",
    },
  );
}

async function start() {
  client = createClient();
  try {
    await client.start();
  } catch (e) {
    const command = vscode.workspace.getConfiguration("lunar").get("server.path") || "lunar";
    const choice = await vscode.window.showErrorMessage(
      `Could not start the Lunar language server (\`${command} lsp\`): ${e.message ?? e}. ` +
        "Install `lunar` on your PATH (cargo install --path crates/ln-driver) or set `lunar.server.path`.",
      "Open Settings",
    );
    if (choice === "Open Settings") {
      vscode.commands.executeCommand("workbench.action.openSettings", "lunar.server.path");
    }
    client = undefined;
  }
}

async function stop() {
  if (client) {
    const c = client;
    client = undefined;
    await c.stop();
  }
}

/** @param {vscode.ExtensionContext} context */
async function activate(context) {
  context.subscriptions.push(
    vscode.commands.registerCommand("lunar.restartServer", async () => {
      await stop();
      await start();
    }),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (e.affectsConfiguration("lunar.server.path")) {
        await stop();
        await start();
      }
    }),
  );
  await start();
}

function deactivate() {
  return stop();
}

module.exports = { activate, deactivate };
