// Lunar for VS Code: starts `lunar lsp` and connects it to .ln files.

const fs = require("fs");
const os = require("os");
const path = require("path");
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

/** @type {LanguageClient | undefined} */
let client;

/**
 * The `lunar` to run: `lunar.server.path` if set, else `lunar` on PATH, else
 * the one the install scripts put in `$LUNAR_PREFIX/bin` or `~/.lunar/bin`
 * (an editor started from the desktop may not see a PATH set in a shell
 * profile), else plain `lunar`.
 */
function serverCommand() {
  const configured = vscode.workspace.getConfiguration("lunar").get("server.path");
  if (configured) {
    return configured;
  }
  const exe = process.platform === "win32" ? "lunar.exe" : "lunar";
  for (const dir of (process.env.PATH || "").split(path.delimiter)) {
    if (dir && fs.existsSync(path.join(dir, exe))) {
      return path.join(dir, exe);
    }
  }
  const prefix = process.env.LUNAR_PREFIX || path.join(os.homedir(), ".lunar");
  const installed = path.join(prefix, "bin", exe);
  return fs.existsSync(installed) ? installed : "lunar";
}

function createClient() {
  const command = serverCommand();
  // No `transport`: stdio is the default, and naming it makes the client
  // append `--stdio` to the arguments.
  const run = { command, args: ["lsp"] };
  return new LanguageClient(
    "lunar",
    "Lunar Language Server",
    { run, debug: run },
    {
      documentSelector: [
        { scheme: "file", language: "lunar" },
        // The standard library, shown as virtual read-only documents.
        { scheme: "lunar-std", language: "lunar" },
      ],
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
    const command = serverCommand();
    const choice = await vscode.window.showErrorMessage(
      `Could not start the Lunar language server (\`${command} lsp\`): ${e.message ?? e}. ` +
        "Install Lunar (see https://github.com/lunarln/lunar#installing) or set `lunar.server.path`.",
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

/**
 * `lunar-std:/core/string.ln` documents: the standard library embedded in the
 * compiler. They have no file on disk, so they are read-only and pathless;
 * the text comes from the language server.
 */
const stdProvider = {
  async provideTextDocumentContent(uri) {
    if (!client) {
      return "// The Lunar language server is not running.\n";
    }
    try {
      // `true`: don't percent-encode (`core::slice`, not `core%3A%3Aslice`).
      return await client.sendRequest("lunar/stdSource", { uri: uri.toString(true) });
    } catch (e) {
      return `// Could not load ${uri.path}: ${e.message ?? e}\n`;
    }
  },
};

/** @param {vscode.ExtensionContext} context */
async function activate(context) {
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider("lunar-std", stdProvider),
    // `lunar-std:core::string` has no `.ln` extension to pick the language.
    vscode.workspace.onDidOpenTextDocument((doc) => {
      if (doc.uri.scheme === "lunar-std" && doc.languageId !== "lunar") {
        vscode.languages.setTextDocumentLanguage(doc, "lunar");
      }
    }),
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
